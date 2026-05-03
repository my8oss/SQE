use std::fs::File;
use std::io::{self, BufRead};
use std::path::Path;
 
use crate::items::{Choose, Insert, Html, Js, Css};

#[derive(Debug)]
pub enum Entry {
    Import {
        path: String,
    },
    /// Document-level title (the big title for the whole questionnaire)
    DocTitle(String),
    Page {
        title: String,
        content: Vec<Question>,
    },
}

#[derive(Debug)]
pub enum Question {
    Choose(Choose),
    Insert(Insert),
    Html(Html),
    Js(Js),
    Css(Css),
}

// --- new helper: reads a brace-delimited block while ignoring braces inside strings ---
fn read_brace_block<I>(
    lines: &mut std::iter::Peekable<I>,
    first_after_open: &str,
) -> io::Result<String>
where
    I: Iterator<Item = io::Result<String>>,
{
    let mut out = String::new();

    // We start *after* the initial '{' (first_after_open is the substring after the first '{' on that line).
    // We'll treat the stream as though we've seen a starting brace, so depth starts at 1.
    let mut depth: i32 = 1;
    let mut in_single = false;
    let mut in_double = false;
    let mut in_backtick = false;
    let mut escape = false;

    // Helper to process a single line's characters
    let mut process_chars = |s: &str| -> Option<()> {
        for ch in s.chars() {
            if escape {
                // previous char was backslash, consume this char literally
                out.push(ch);
                escape = false;
                continue;
            }

            match ch {
                '\\' => {
                    // begin escape sequence
                    out.push(ch);
                    escape = true;
                }
                '\'' => {
                    out.push(ch);
                    if !in_double && !in_backtick {
                        in_single = !in_single;
                    }
                }
                '"' => {
                    out.push(ch);
                    if !in_single && !in_backtick {
                        in_double = !in_double;
                    }
                }
                '`' => {
                    out.push(ch);
                    if !in_single && !in_double {
                        in_backtick = !in_backtick;
                    }
                }
                '{' => {
                    if !in_single && !in_double && !in_backtick {
                        depth += 1;
                        out.push(ch);
                    } else {
                        out.push(ch);
                    }
                }
                '}' => {
                    if !in_single && !in_double && !in_backtick {
                        depth -= 1;
                        if depth == 0 {
                            // we reached the matching closing brace — DONE (do NOT include this brace)
                            return None; // signal done for caller to stop
                        } else {
                            out.push(ch);
                        }
                    } else {
                        out.push(ch);
                    }
                }
                _ => {
                    out.push(ch);
                }
            }
        }
        // still more to read
        out.push('\n');
        Some(())
    };

    // process the remainder of the line after the opening brace first
    if !first_after_open.is_empty() {
        if process_chars(first_after_open).is_none() {
            return Ok(out);
        }
    }

    // then continue reading subsequent lines until depth returns to 0 or EOF
    while let Some(line_res) = lines.next() {
        let line = line_res?;
        if process_chars(&line).is_none() {
            return Ok(out);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::UnexpectedEof,
        "unterminated brace block",
    ))
}

// --- old parse_block kept for compatibility when simple closing marker is desired ---
// (You can keep it or remove it. It's still used nowhere after this change.)
pub fn parse_block<I>(
    lines: &mut std::iter::Peekable<I>,
    closing_marker: &str,
) -> io::Result<String>
where
    I: Iterator<Item = io::Result<String>>,
{
    let mut out = String::new();
    while let Some(line_res) = lines.next() {
        let line = line_res?;
        if line.contains(closing_marker) {
            if let Some(pos) = line.find(closing_marker) {
                let before = &line[..pos];
                if !before.trim().is_empty() {
                    out.push_str(before);
                    out.push('\n');
                }
            }
            break;
        } else {
            out.push_str(&line);
            out.push('\n');
        }
    }
    Ok(out)
}

// ... then the compile() function follows but with updated block handling ...
pub fn compile<P: AsRef<Path>>(path: P) -> io::Result<Vec<Entry>> {
    let file = File::open(path)?;
    let reader = io::BufReader::new(file);
    let mut lines_iter = reader.lines().peekable();

    let mut ast: Vec<Entry> = Vec::new();
    let mut current_page: Option<(String, Vec<Question>)> = None;

    while let Some(line_res) = lines_iter.next() {
        let raw = line_res?;
        let line = raw.trim();

        if line.is_empty() || line.starts_with("//") {
            continue;
        }
 
        // Support a document-level `title` directive. This sets the questionnaire-wide title
        // and does NOT create or modify the current page. Use @p for per-page titles.
        if line.starts_with("title") {
            // use the trimmed `line` (not raw) so leading whitespace is handled consistently
            let rest = line["title".len()..].trim();
            let title = if rest.starts_with('"') {
                rest.split('"').nth(1).unwrap_or("untitled").to_string()
            } else if !rest.is_empty() {
                rest.to_string()
            } else {
                "untitled".to_string()
            };
            // Emit a document-level title entry into the AST so the converter can use it.
            ast.push(Entry::DocTitle(title));
            continue;
        }
 
        if line.starts_with("@p") {
            // Support both quoted and unquoted page titles:
            //   @p "Page One"
            //   @p Page One
            let rest = line["@p".len()..].trim();
            let page_title = if rest.starts_with('"') {
                rest.split('"').nth(1).unwrap_or("untitled").to_string()
            } else if !rest.is_empty() {
                rest.to_string()
            } else {
                "untitled".to_string()
            };
 
            // If there's an existing current_page and its title is the placeholder "untitled",
            // adopt the @p title for that page so content collected before the first @p gets the proper title.
            if let Some((cur_title, _cur_content)) = current_page.as_mut() {
                if cur_title == "untitled" {
                    *cur_title = page_title;
                    continue;
                } else {
                    // Close the current page and start a new one with the provided title.
                    if let Some((t, c)) = current_page.take() {
                        ast.push(Entry::Page { title: t, content: c });
                    }
                    current_page = Some((page_title, Vec::new()));
                    continue;
                }
            } else {
                // No current page: start one with the provided title.
                current_page = Some((page_title, Vec::new()));
                continue;
            }
        }

        if line.starts_with("import") {
            let path = line.split('"').nth(1).unwrap_or("").to_string();
            ast.push(Entry::Import { path });
            continue;
        }

        if line.starts_with("insert") {
            // handle inline { ... } and block { ... } with brace-aware parser
            if let Some(open_pos) = raw.find('{') {
                // take substring after first '{'
                let after = &raw[open_pos + 1..];
                let block = read_brace_block(&mut lines_iter, after)?;
                let text = block; // preserve as-is (block already contains newlines)
                let insert_node = Insert::parse(&text);
                if let Some((_title, content)) = current_page.as_mut() {
                    content.push(Question::Insert(insert_node));
                } else {
                    // No current page — start one instead of creating a standalone page entry.
                    current_page = Some(("untitled".to_string(), vec![Question::Insert(insert_node)]));
                }
                continue;
            }
 
            // fallback: single-line insert without braces
            let words: Vec<&str> = line.split_whitespace().collect();
            if words.len() > 1 {
                let text = words[1..].join(" ");
                let insert_node = Insert::parse(&text);
                if let Some((_title, content)) = current_page.as_mut() {
                    content.push(Question::Insert(insert_node));
                } else {
                    // Start a current page so subsequent items are grouped with it
                    current_page = Some(("untitled".to_string(), vec![Question::Insert(insert_node)]));
                }
            }
            continue;
        }

        if line.starts_with("choice") {
            let mut id: Option<String> = None;
            if line.contains('{') {
                let before_brace = line.split('{').next().unwrap_or("");
                let parts: Vec<&str> = before_brace.split_whitespace().collect();
                if parts.len() >= 2 {
                    id = Some(parts[1].to_string());
                }
            } else {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    id = Some(parts[1].to_string());
                }
            }

            // get substring after first '{', if any
            let block = if let Some(open_pos) = raw.find('{') {
                let after = &raw[open_pos + 1..];
                read_brace_block(&mut lines_iter, after)?
            } else {
                // block starts on following lines
                read_brace_block(&mut lines_iter, "")?
            };

            let choose_node = Choose::parse(&block, id);

            if let Some((_title, content)) = current_page.as_mut() {
                content.push(Question::Choose(choose_node));
            } else {
                // Start a new current page when none exists.
                current_page = Some(("untitled".to_string(), vec![Question::Choose(choose_node)]));
            }

            continue;
        }

        if line.starts_with("html") {
            let block = if let Some(open_pos) = raw.find('{') {
                let after = &raw[open_pos + 1..];
                read_brace_block(&mut lines_iter, after)?
            } else {
                read_brace_block(&mut lines_iter, "")?
            };
 
            let node = Html::parse(&block);
            if let Some((_title, content)) = current_page.as_mut() {
                content.push(Question::Html(node));
            } else {
                current_page = Some(("untitled".to_string(), vec![Question::Html(node)]));
            }
            continue;
        }
 
        if line.starts_with("js") {
            let block = if let Some(open_pos) = raw.find('{') {
                let after = &raw[open_pos + 1..];
                read_brace_block(&mut lines_iter, after)?
            } else {
                read_brace_block(&mut lines_iter, "")?
            };
 
            let node = Js::parse(&block);
            if let Some((_title, content)) = current_page.as_mut() {
                content.push(Question::Js(node));
            } else {
                current_page = Some(("untitled".to_string(), vec![Question::Js(node)]));
            }
            continue;
        }
 
        if line.starts_with("css") {
            let block = if let Some(open_pos) = raw.find('{') {
                let after = &raw[open_pos + 1..];
                read_brace_block(&mut lines_iter, after)?
            } else {
                read_brace_block(&mut lines_iter, "")?
            };
 
            let node = Css::parse(&block);
            if let Some((_title, content)) = current_page.as_mut() {
                content.push(Question::Css(node));
            } else {
                current_page = Some(("untitled".to_string(), vec![Question::Css(node)]));
            }
            continue;
        }
 

    }

    if let Some((title, content)) = current_page.take() {
        ast.push(Entry::Page { title, content });
    }

    Ok(ast)
}
