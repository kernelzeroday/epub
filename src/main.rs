use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process;

use anyhow::{Context, Result};
use clap::Parser;
use quick_xml::events::Event;

#[derive(Parser)]
#[command(name = "epub", about = "Non-interactive EPUB text extraction")]
struct Cli {
    /// Path to EPUB file
    file: PathBuf,

    /// Chapter(s) to extract: single (3), range (1-5), list (1,3,5), or mixed (1-3,7,9-12)
    #[arg(short, long)]
    chapters: Option<String>,

    /// List chapters with indices
    #[arg(short, long)]
    list: bool,

    /// Show book metadata
    #[arg(short, long)]
    info: bool,
}

struct Book {
    metadata: Metadata,
    toc: Vec<TocEntry>,
    spine: Vec<SpineItem>,
}

#[derive(Default)]
struct Metadata {
    title: Option<String>,
    author: Option<String>,
    publisher: Option<String>,
    language: Option<String>,
}

struct TocEntry {
    label: String,
    href: String,
}

struct SpineItem {
    href: String,
    content: String,
}

fn parse_book(path: &Path) -> Result<Book> {
    let file = std::fs::File::open(path)
        .with_context(|| format!("failed to open {}", path.display()))?;
    let mut zip = zip::ZipArchive::new(file)
        .with_context(|| format!("not a valid EPUB/ZIP: {}", path.display()))?;

    let opf_path = find_opf_path(&mut zip)?;
    let opf_content = read_zip_text(&mut zip, &opf_path)?;
    let (metadata, manifest, spine_ids, ncx_id) = parse_opf(&opf_content)?;

    let opf_dir = Path::new(&opf_path)
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();

    let toc = if let Some(ncx_id) = ncx_id {
        if let Some(ncx_href) = manifest.get(&ncx_id) {
            let ncx_path = resolve_path(&opf_dir, ncx_href);
            let ncx_content = read_zip_text(&mut zip, &ncx_path)?;
            parse_ncx(&ncx_content)?
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    let mut spine = Vec::new();
    for id in &spine_ids {
        if let Some(href) = manifest.get(id) {
            let full_path = resolve_path(&opf_dir, href);
            match read_zip_text(&mut zip, &full_path) {
                Ok(html) => {
                    let text = extract_text(&html);
                    spine.push(SpineItem {
                        href: href.clone(),
                        content: text,
                    });
                }
                Err(_) => {
                    spine.push(SpineItem {
                        href: href.clone(),
                        content: String::new(),
                    });
                }
            }
        }
    }

    Ok(Book { metadata, toc, spine })
}

fn find_opf_path<R: Read + std::io::Seek>(zip: &mut zip::ZipArchive<R>) -> Result<String> {
    let container = read_zip_text(zip, "META-INF/container.xml")
        .context("missing META-INF/container.xml")?;
    let mut reader = quick_xml::Reader::from_str(&container);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Empty(ref e)) | Ok(Event::Start(ref e)) => {
                if e.name().as_ref() == b"rootfile" {
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"full-path" {
                            return Ok(attr
                                .decode_and_unescape_value(reader.decoder())?
                                .to_string());
                        }
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => anyhow::bail!("XML error in container.xml: {e}"),
            _ => {}
        }
        buf.clear();
    }
    anyhow::bail!("no rootfile found in container.xml")
}

fn parse_opf(
    content: &str,
) -> Result<(Metadata, HashMap<String, String>, Vec<String>, Option<String>)> {
    let mut reader = quick_xml::Reader::from_str(content);
    let mut metadata = Metadata::default();
    let mut manifest: HashMap<String, String> = HashMap::new();
    let mut spine: Vec<String> = Vec::new();
    let mut ncx_id: Option<String> = None;
    let mut current_tag: Option<String> = None;
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if name.contains("title") {
                    current_tag = Some("title".into());
                } else if name.contains("creator") {
                    current_tag = Some("author".into());
                } else if name.contains("publisher") {
                    current_tag = Some("publisher".into());
                } else if name.contains("language") {
                    current_tag = Some("language".into());
                } else if name == "spine" {
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"toc" {
                            ncx_id =
                                Some(attr.decode_and_unescape_value(reader.decoder())?.to_string());
                        }
                    }
                }
            }
            Ok(Event::Empty(ref e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if name.contains("item") && !name.contains("itemref") {
                    let mut id = String::new();
                    let mut href = String::new();
                    for attr in e.attributes().flatten() {
                        match attr.key.as_ref() {
                            b"id" => {
                                id = attr
                                    .decode_and_unescape_value(reader.decoder())?
                                    .to_string();
                            }
                            b"href" => {
                                href = attr
                                    .decode_and_unescape_value(reader.decoder())?
                                    .to_string();
                            }
                            _ => {}
                        }
                    }
                    if !id.is_empty() && !href.is_empty() {
                        manifest.insert(id, href);
                    }
                } else if name.contains("itemref") {
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"idref" {
                            spine.push(
                                attr.decode_and_unescape_value(reader.decoder())?.to_string(),
                            );
                            break;
                        }
                    }
                }
            }
            Ok(Event::Text(e)) => {
                if let Some(tag) = current_tag.take() {
                    let text = e
                        .unescape()
                        .unwrap_or_else(|_| {
                            std::str::from_utf8(e.as_ref()).unwrap_or_default().into()
                        })
                        .trim()
                        .to_string();
                    if !text.is_empty() {
                        match tag.as_str() {
                            "title" => metadata.title = Some(text),
                            "author" => metadata.author = Some(text),
                            "publisher" => metadata.publisher = Some(text),
                            "language" => metadata.language = Some(text),
                            _ => {}
                        }
                    }
                }
            }
            Ok(Event::End(_)) => {
                current_tag = None;
            }
            Ok(Event::Eof) => break,
            Err(e) => anyhow::bail!("XML error in OPF: {e}"),
            _ => {}
        }
        buf.clear();
    }

    Ok((metadata, manifest, spine, ncx_id))
}

fn parse_ncx(content: &str) -> Result<Vec<TocEntry>> {
    let mut reader = quick_xml::Reader::from_str(content);
    let mut entries = Vec::new();
    let mut buf = Vec::new();
    let mut label = String::new();
    let mut href = String::new();
    let mut in_nav_point = 0u32;
    let mut in_nav_label = false;
    let mut in_text = false;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                let name = e.name();
                match name.as_ref() {
                    b"navPoint" => {
                        in_nav_point += 1;
                        if in_nav_point == 1 {
                            label.clear();
                            href.clear();
                        }
                    }
                    b"navLabel" => in_nav_label = true,
                    b"text" if in_nav_label => in_text = true,
                    _ => {}
                }
            }
            Ok(Event::End(ref e)) => {
                let name = e.name();
                match name.as_ref() {
                    b"navPoint" => {
                        if in_nav_point == 1 && !label.is_empty() {
                            entries.push(TocEntry {
                                label: label.clone(),
                                href: href.clone(),
                            });
                        }
                        in_nav_point = in_nav_point.saturating_sub(1);
                    }
                    b"navLabel" => in_nav_label = false,
                    b"text" => in_text = false,
                    _ => {}
                }
            }
            Ok(Event::Empty(ref e)) => {
                if e.name().as_ref() == b"content" && in_nav_point == 1 {
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"src" {
                            href = attr
                                .decode_and_unescape_value(reader.decoder())?
                                .to_string();
                        }
                    }
                }
            }
            Ok(Event::Text(e)) => {
                if in_text && in_nav_point == 1 {
                    label = e
                        .unescape()
                        .unwrap_or_else(|_| {
                            std::str::from_utf8(e.as_ref()).unwrap_or_default().into()
                        })
                        .trim()
                        .to_string();
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => anyhow::bail!("XML error in NCX: {e}"),
            _ => {}
        }
        buf.clear();
    }

    Ok(entries)
}

fn extract_text(html: &str) -> String {
    let mut reader = quick_xml::Reader::from_str(html);
    let mut text = String::new();
    let mut buf = Vec::new();
    let mut skip = false;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                let tag = e.name();
                match tag.as_ref() {
                    b"script" | b"style" | b"head" => skip = true,
                    b"p" | b"div" | b"br" | b"li" | b"h1" | b"h2" | b"h3" | b"h4" | b"h5"
                    | b"h6" | b"blockquote" | b"tr" => {
                        if !text.is_empty() && !text.ends_with('\n') {
                            text.push('\n');
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::End(ref e)) => match e.name().as_ref() {
                b"script" | b"style" | b"head" => skip = false,
                b"p" | b"div" | b"li" | b"h1" | b"h2" | b"h3" | b"h4" | b"h5" | b"h6"
                | b"blockquote" | b"tr" => {
                    if !text.ends_with('\n') {
                        text.push('\n');
                    }
                }
                _ => {}
            },
            Ok(Event::Text(e)) => {
                if !skip {
                    let unescaped = e.unescape().unwrap_or_else(|_| {
                        std::str::from_utf8(e.as_ref()).unwrap_or_default().into()
                    });
                    let clean: String = unescaped.chars().filter(|c| !c.is_control()).collect();
                    let trimmed = clean.trim();
                    if !trimmed.is_empty() {
                        if !text.is_empty()
                            && !text.ends_with('\n')
                            && !text.ends_with(' ')
                        {
                            text.push(' ');
                        }
                        text.push_str(trimmed);
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }

    text.lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn read_zip_text<R: Read + std::io::Seek>(
    zip: &mut zip::ZipArchive<R>,
    name: &str,
) -> Result<String> {
    let mut file = zip.by_name(name).with_context(|| format!("missing {name}"))?;
    let mut buf = String::new();
    file.read_to_string(&mut buf)?;
    Ok(buf)
}

fn resolve_path(base_dir: &str, href: &str) -> String {
    if base_dir.is_empty() {
        return href.to_string();
    }
    let resolved = PathBuf::from(base_dir).join(href);
    resolved.to_string_lossy().replace('\\', "/")
}

fn href_base(href: &str) -> &str {
    href.split('#').next().unwrap_or(href)
}

fn resolve_toc_to_spine(book: &Book, toc_indices: &[usize]) -> Vec<usize> {
    let mut spine_indices = Vec::new();
    for &ti in toc_indices {
        let entry = &book.toc[ti];
        let base = href_base(&entry.href);
        let next_base = book.toc.get(ti + 1).map(|e| href_base(&e.href));

        let start = book.spine.iter().position(|s| href_base(&s.href) == base);
        let end = next_base.and_then(|nb| book.spine.iter().position(|s| href_base(&s.href) == nb));

        if let Some(start) = start {
            let end = end.unwrap_or(start + 1);
            for i in start..end {
                if !spine_indices.contains(&i) {
                    spine_indices.push(i);
                }
            }
        }
    }
    spine_indices
}

fn parse_chapter_spec(spec: &str, max: usize) -> Result<Vec<usize>> {
    let mut indices = Vec::new();
    for part in spec.split(',') {
        let part = part.trim();
        if let Some((start, end)) = part.split_once('-') {
            let s: usize = start.trim().parse().context("invalid range start")?;
            let e: usize = end.trim().parse().context("invalid range end")?;
            if s == 0 || e == 0 {
                anyhow::bail!("chapter indices start at 1");
            }
            if s > e {
                anyhow::bail!("invalid range: {s}-{e}");
            }
            for i in s..=e.min(max) {
                indices.push(i - 1);
            }
        } else {
            let i: usize = part.parse().context("invalid chapter number")?;
            if i == 0 {
                anyhow::bail!("chapter indices start at 1");
            }
            if i > max {
                anyhow::bail!("chapter {i} out of range (book has {max} chapters)");
            }
            indices.push(i - 1);
        }
    }
    Ok(indices)
}

fn cmd_info(book: &Book) {
    let m = &book.metadata;
    if let Some(title) = &m.title {
        println!("Title:     {title}");
    }
    if let Some(author) = &m.author {
        println!("Author:    {author}");
    }
    if let Some(publisher) = &m.publisher {
        println!("Publisher: {publisher}");
    }
    if let Some(language) = &m.language {
        println!("Language:  {language}");
    }
    println!("Chapters:  {}", book.toc.len());
    println!("Pages:     {}", book.spine.len());
}

fn cmd_list(book: &Book) {
    if book.toc.is_empty() {
        println!("(no TOC; {} spine items)", book.spine.len());
        for (i, item) in book.spine.iter().enumerate() {
            let len = item.content.len();
            println!("  {:>3}  {} ({len} bytes)", i + 1, item.href);
        }
    } else {
        for (i, entry) in book.toc.iter().enumerate() {
            println!("{:>3}  {}", i + 1, entry.label);
        }
    }
}

fn cmd_read(book: &Book, chapters: Option<&str>) -> Result<()> {
    let spine_indices = match chapters {
        Some(spec) => {
            if book.toc.is_empty() {
                let indices = parse_chapter_spec(spec, book.spine.len())?;
                indices
            } else {
                let toc_indices = parse_chapter_spec(spec, book.toc.len())?;
                resolve_toc_to_spine(book, &toc_indices)
            }
        }
        None => {
            (0..book.spine.len())
                .filter(|&i| !book.spine[i].content.is_empty())
                .collect()
        }
    };

    if spine_indices.is_empty() {
        if let Some(spec) = chapters {
            eprintln!("epub: no content found for chapter(s) {spec}");
        }
        return Ok(());
    }

    let all_empty = spine_indices
        .iter()
        .all(|&i| book.spine.get(i).is_none_or(|s| s.content.is_empty()));

    if all_empty {
        if let Some(spec) = chapters {
            eprintln!(
                "epub: chapter(s) {spec} resolved but contained no text (image-only pages)"
            );
        }
        return Ok(());
    }

    let mut first = true;
    for &idx in &spine_indices {
        if let Some(item) = book.spine.get(idx) {
            if !item.content.is_empty() {
                if !first {
                    println!();
                }
                print!("{}", item.content);
                first = false;
            }
        }
    }
    if !first {
        println!();
    }
    Ok(())
}

fn main() {
    let cli = Cli::parse();
    let book = match parse_book(&cli.file) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("epub: {e:#}");
            process::exit(1);
        }
    };

    let result = if cli.info {
        cmd_info(&book);
        Ok(())
    } else if cli.list {
        cmd_list(&book);
        Ok(())
    } else {
        cmd_read(&book, cli.chapters.as_deref())
    };

    if let Err(e) = result {
        eprintln!("epub: {e:#}");
        process::exit(1);
    }
}
