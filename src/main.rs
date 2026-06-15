use std::path::{Path, PathBuf};
use std::process;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use epub_parser::Epub;

#[derive(Parser)]
#[command(name = "epubcat", about = "Non-interactive EPUB text extraction")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show book metadata
    Info {
        /// Path to EPUB file
        file: PathBuf,
    },
    /// List chapters with indices
    List {
        /// Path to EPUB file
        file: PathBuf,
    },
    /// Extract chapter text to stdout
    Read {
        /// Path to EPUB file
        file: PathBuf,
        /// Chapter(s) to extract: single (3), range (1-5), list (1,3,5), or mixed (1-3,7,9-12)
        #[arg(short, long)]
        chapters: Option<String>,
    },
}

fn parse_epub(path: &Path) -> Result<Epub> {
    Epub::parse(path).with_context(|| format!("failed to parse {}", path.display()))
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
                anyhow::bail!("chapter {i} out of range (book has {max} pages)");
            }
            indices.push(i - 1);
        }
    }
    Ok(indices)
}

fn cmd_info(path: &Path) -> Result<()> {
    let epub = parse_epub(path)?;
    let m = &epub.metadata;
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
    println!("Chapters:  {}", epub.toc.len());
    println!("Pages:     {}", epub.pages.len());
    Ok(())
}

fn cmd_list(path: &Path) -> Result<()> {
    let epub = parse_epub(path)?;
    if epub.toc.is_empty() {
        println!("(no TOC entries; book has {} pages)", epub.pages.len());
        for page in &epub.pages {
            println!("  {:>3}  (page {})", page.index + 1, page.index);
        }
    } else {
        for (i, entry) in epub.toc.iter().enumerate() {
            println!("{:>3}  {}", i + 1, entry.label);
        }
    }
    Ok(())
}

fn cmd_read(path: &Path, chapters: Option<&str>) -> Result<()> {
    let epub = parse_epub(path)?;
    if epub.pages.is_empty() {
        anyhow::bail!("no readable pages found in {}", path.display());
    }

    let indices = match chapters {
        Some(spec) => parse_chapter_spec(spec, epub.pages.len())?,
        None => (0..epub.pages.len()).collect(),
    };

    for (pos, &idx) in indices.iter().enumerate() {
        if let Some(page) = epub.pages.get(idx) {
            if pos > 0 {
                println!();
            }
            print!("{}", page.content);
        }
    }
    Ok(())
}

fn main() {
    let cli = Cli::parse();
    let result = match &cli.command {
        Command::Info { file } => cmd_info(file),
        Command::List { file } => cmd_list(file),
        Command::Read { file, chapters } => cmd_read(file, chapters.as_deref()),
    };
    if let Err(e) = result {
        eprintln!("epubcat: {e:#}");
        process::exit(1);
    }
}
