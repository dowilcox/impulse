//! Turn conflicted files into a prompt asking a coding agent to resolve
//! them: each conflict block with its line numbers, size-capped.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    /// 1-based lines of the `<<<<<<<` and `>>>>>>>` markers.
    pub start_line: usize,
    pub end_line: usize,
    pub text: String,
}

/// How many characters of conflict blocks a prompt shows by default.
pub const DEFAULT_LIMIT: usize = 24_000;

/// Conflict blocks in a file's text (`<<<<<<<` through `>>>>>>>`, including
/// a diff3 `|||||||` base section).
pub fn blocks(text: &str) -> Vec<Block> {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut blocks = Vec::new();
    let mut start: Option<usize> = None;
    for (index, line) in lines.iter().enumerate() {
        if line.starts_with("<<<<<<<") {
            start = Some(index);
        } else if line.starts_with(">>>>>>>") {
            if let Some(first) = start.take() {
                blocks.push(Block {
                    start_line: first + 1,
                    end_line: index + 1,
                    text: lines[first..=index].join("\n"),
                });
            }
        }
    }
    blocks
}

/// The prompt for `files` (path relative to the repository, contents).
/// `operation` names what's in progress ("merge", "rebase", ...). Blocks
/// past `limit` characters are listed by line only.
pub fn make(files: &[(String, String)], operation: Option<&str>, limit: usize) -> String {
    let mut out = String::from("Please resolve the merge conflicts");
    if let Some(operation) = operation.filter(|o| !o.is_empty()) {
        out += &format!(" from this {operation}");
    }
    out += " in the files below. Keep the intent of both sides where it makes sense, remove the \
            conflict markers, and make sure the result builds. Don't stage or commit anything.\n";
    let mut used = out.chars().count();
    let mut omitted: Vec<String> = Vec::new();
    for (path, text) in files {
        let file_blocks = blocks(text);
        if file_blocks.is_empty() {
            continue;
        }
        let plural = if file_blocks.len() == 1 { "" } else { "s" };
        out += &format!("\n## {path} ({} conflict{plural})\n", file_blocks.len());
        for block in file_blocks {
            let fence = if block.text.contains("```") {
                "````"
            } else {
                "```"
            };
            let section = format!(
                "\nLines {}–{}:\n{fence}\n{}\n{fence}\n",
                block.start_line, block.end_line, block.text
            );
            let size = section.chars().count();
            if used + size > limit {
                omitted.push(format!("{path}:{}-{}", block.start_line, block.end_line));
                continue;
            }
            out += &section;
            used += size;
        }
    }
    if !omitted.is_empty() {
        out += &format!(
            "\nAlso resolve these (not shown, open the files): {}\n",
            omitted.join(", ")
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "import A
<<<<<<< HEAD
let x = 1
=======
let x = 2
>>>>>>> topic
print(x)
<<<<<<< HEAD
a()
||||||| base
b()
=======
c()
>>>>>>> topic";

    #[test]
    fn finds_blocks_with_their_lines() {
        let found = blocks(FILE);
        assert_eq!(
            found.iter().map(|b| b.start_line).collect::<Vec<_>>(),
            [2, 8]
        );
        assert_eq!(
            found.iter().map(|b| b.end_line).collect::<Vec<_>>(),
            [6, 14]
        );
        assert_eq!(
            found[0].text,
            "<<<<<<< HEAD\nlet x = 1\n=======\nlet x = 2\n>>>>>>> topic"
        );
        assert!(blocks("no markers\n").is_empty());
        assert!(blocks("<<<<<<< HEAD\nunterminated").is_empty());
    }

    #[test]
    fn prompt_lists_every_conflict() {
        let files = [
            ("src/a.swift".to_string(), FILE.to_string()),
            ("clean.txt".to_string(), "fine\n".to_string()),
        ];
        let prompt = make(&files, Some("rebase"), DEFAULT_LIMIT);
        assert!(prompt.starts_with("Please resolve the merge conflicts from this rebase"));
        assert!(prompt.contains("## src/a.swift (2 conflicts)"));
        assert!(prompt.contains("Lines 2–6:\n```\n<<<<<<< HEAD\nlet x = 1"));
        assert!(prompt.contains("Lines 8–14:"));
        assert!(
            !prompt.contains("clean.txt"),
            "files without markers are skipped"
        );
    }

    #[test]
    fn oversized_blocks_are_listed_by_line() {
        let prompt = make(&[("big.txt".to_string(), FILE.to_string())], None, 330);
        assert!(prompt.contains("Lines 2–6:"));
        assert!(prompt.contains("not shown, open the files): big.txt:8-14"));
    }
}
