//! mdBook preprocessor for file inclusion with shift
//!
//! Based on the links preprocessor in the main mdBook project.

use anyhow::Context;
use clap::{App, Arg, SubCommand};
use log::{error, warn};
use mdbook::{
    book::{Book, BookItem},
    errors::{Error, Result},
    preprocess::{CmdPreprocessor, Preprocessor, PreprocessorContext},
};
use once_cell::sync::Lazy;
use regex::{CaptureMatches, Captures, Regex};
use std::{
    fs, io,
    ops::{Bound, Range, RangeBounds, RangeFrom, RangeFull, RangeTo},
    path::{Path, PathBuf},
    process,
};

mod string;
use string::{take_anchored_lines, take_lines};

const ESCAPE_CHAR: char = '\\';
const MAX_LINK_NESTED_DEPTH: usize = 10;

fn main() -> Result<(), Error> {
    env_logger::init();
    let app = App::new(ShiftInclude::NAME)
        .about("An mdbook preprocessor which includes files with shift")
        .subcommand(
            SubCommand::with_name("supports")
                .arg(Arg::with_name("renderer").required(true))
                .about("Check whether a renderer is supported by this preprocessor"),
        );
    let matches = app.get_matches();

    if let Some(sub_args) = matches.subcommand_matches("supports") {
        let renderer = sub_args.value_of("renderer").expect("Required argument");
        let supported = ShiftInclude::supports_renderer(renderer);

        // Signal whether the renderer is supported by exiting with 1 or 0.
        if supported {
            process::exit(0);
        } else {
            process::exit(1);
        }
    } else {
        let (ctx, book) = CmdPreprocessor::parse_input(io::stdin())?;
        let pre = ShiftInclude::new(&ctx);

        let processed_book = pre.run(&ctx, book)?;
        serde_json::to_writer(io::stdout(), &processed_book)?;
    }
    Ok(())
}

/// A pre-processor that acts like `{{#include}}` but allows shifting.
#[derive(Default)]
pub struct ShiftInclude;

impl ShiftInclude {
    const NAME: &'static str = "shiftinclude";

    fn new(ctx: &PreprocessorContext) -> Self {
        if ctx.mdbook_version != mdbook::MDBOOK_VERSION {
            // We should probably use the `semver` crate to check compatibility
            // here...
            warn!(
                "The {} plugin was built against version {} of mdbook, \
             but we're being called from version {}",
                Self::NAME,
                mdbook::MDBOOK_VERSION,
                ctx.mdbook_version
            );
        }
        Self
    }

    /// Indicate whether a renderer is supported.  This preprocessor can emit MarkDown so should support almost any
    /// renderer.
    fn supports_renderer(renderer: &str) -> bool {
        renderer != "not-supported"
    }
}

impl Preprocessor for ShiftInclude {
    fn name(&self) -> &str {
        Self::NAME
    }

    fn run(&self, ctx: &PreprocessorContext, mut book: Book) -> Result<Book, Error> {
        let src_dir = ctx.root.join(&ctx.config.book.src);

        book.for_each_mut(|section: &mut BookItem| {
            if let BookItem::Chapter(ch) = section {
                if let Some(chapter_path) = &ch.path {
                    let base = chapter_path
                        .parent()
                        .map(|dir| src_dir.join(dir))
                        .expect("All book items have a parent");

                    let content = replace_all(&ch.content, base, chapter_path, 0);
                    ch.content = content;
                }
            }
        });
        Ok(book)
    }
}

fn replace_all<P1, P2>(s: &str, path: P1, source: P2, depth: usize) -> String
where
    P1: AsRef<Path>,
    P2: AsRef<Path>,
{
    // When replacing one thing in a string by something with a different length,
    // the indices after that will not correspond,
    // we therefore have to store the difference to correct this
    let path = path.as_ref();
    let source = source.as_ref();
    let mut previous_end_index = 0;
    let mut replaced = String::new();

    for link in find_links(s) {
        replaced.push_str(&s[previous_end_index..link.start_index]);

        match link.render_with_path(path) {
            Ok(new_content) => {
                if depth < MAX_LINK_NESTED_DEPTH {
                    if let Some(rel_path) = link.link_type.relative_path(path) {
                        replaced.push_str(&replace_all(&new_content, rel_path, source, depth + 1));
                    } else {
                        replaced.push_str(&new_content);
                    }
                } else {
                    error!(
                        "Stack depth exceeded in {}. Check for cyclic includes",
                        source.display()
                    );
                }
                previous_end_index = link.end_index;
            }
            Err(e) => {
                error!("Error updating \"{}\", {}", link.link_text, e);
                for cause in e.chain().skip(1) {
                    warn!("Caused By: {}", cause);
                }

                // This should make sure we include the raw `{{# ... }}` snippet
                // in the page content if there are any errors.
                previous_end_index = link.start_index;
            }
        }
    }

    replaced.push_str(&s[previous_end_index..]);
    replaced
}

#[derive(PartialEq, Debug, Clone)]
enum LinkType {
    Escaped,
    Include(PathBuf, RangeOrAnchor),
}

#[derive(PartialEq, Debug, Clone)]
enum RangeOrAnchor {
    Range(LineRange),
    Anchor(String),
}

// A range of lines specified with some include directive.
#[allow(clippy::enum_variant_names)] // The prefix can't be removed, and is meant to mirror the contained type
#[derive(PartialEq, Debug, Clone)]
enum LineRange {
    Range(Range<usize>),
    RangeFrom(RangeFrom<usize>),
    RangeTo(RangeTo<usize>),
    RangeFull(RangeFull),
}

impl RangeBounds<usize> for LineRange {
    fn start_bound(&self) -> Bound<&usize> {
        match self {
            LineRange::Range(r) => r.start_bound(),
            LineRange::RangeFrom(r) => r.start_bound(),
            LineRange::RangeTo(r) => r.start_bound(),
            LineRange::RangeFull(r) => r.start_bound(),
        }
    }

    fn end_bound(&self) -> Bound<&usize> {
        match self {
            LineRange::Range(r) => r.end_bound(),
            LineRange::RangeFrom(r) => r.end_bound(),
            LineRange::RangeTo(r) => r.end_bound(),
            LineRange::RangeFull(r) => r.end_bound(),
        }
    }
}

impl From<Range<usize>> for LineRange {
    fn from(r: Range<usize>) -> LineRange {
        LineRange::Range(r)
    }
}

impl From<RangeFrom<usize>> for LineRange {
    fn from(r: RangeFrom<usize>) -> LineRange {
        LineRange::RangeFrom(r)
    }
}

impl From<RangeTo<usize>> for LineRange {
    fn from(r: RangeTo<usize>) -> LineRange {
        LineRange::RangeTo(r)
    }
}

impl From<RangeFull> for LineRange {
    fn from(r: RangeFull) -> LineRange {
        LineRange::RangeFull(r)
    }
}

impl LinkType {
    fn relative_path<P: AsRef<Path>>(self, base: P) -> Option<PathBuf> {
        let base = base.as_ref();
        match self {
            LinkType::Escaped => None,
            LinkType::Include(p, _) => Some(return_relative_path(base, &p)),
        }
    }
}
fn return_relative_path<P: AsRef<Path>>(base: P, relative: P) -> PathBuf {
    base.as_ref()
        .join(relative)
        .parent()
        .expect("Included file should not be /")
        .to_path_buf()
}

fn parse_range_or_anchor(parts: Option<&str>) -> RangeOrAnchor {
    let mut parts = parts.unwrap_or("").splitn(3, ':').fuse();

    let next_element = parts.next();
    let start = if let Some(value) = next_element.and_then(|s| s.parse::<usize>().ok()) {
        // subtract 1 since line numbers usually begin with 1
        Some(value.saturating_sub(1))
    } else if let Some("") = next_element {
        None
    } else if let Some(anchor) = next_element {
        return RangeOrAnchor::Anchor(String::from(anchor));
    } else {
        None
    };

    let end = parts.next();
    // If `end` is empty string or any other value that can't be parsed as a usize, treat this
    // include as a range with only a start bound. However, if end isn't specified, include only
    // the single line specified by `start`.
    let end = end.map(|s| s.parse::<usize>());

    match (start, end) {
        (Some(start), Some(Ok(end))) => RangeOrAnchor::Range(LineRange::from(start..end)),
        (Some(start), Some(Err(_))) => RangeOrAnchor::Range(LineRange::from(start..)),
        (Some(start), None) => RangeOrAnchor::Range(LineRange::from(start..start + 1)),
        (None, Some(Ok(end))) => RangeOrAnchor::Range(LineRange::from(..end)),
        (None, None) | (None, Some(Err(_))) => RangeOrAnchor::Range(LineRange::from(RangeFull)),
    }
}

fn parse_include_path(path: &str) -> LinkType {
    let mut parts = path.splitn(2, ':');

    let path = parts.next().unwrap().into();
    let range_or_anchor = parse_range_or_anchor(parts.next());

    LinkType::Include(path, range_or_anchor)
}

#[derive(PartialEq, Debug, Clone)]
struct Link<'a> {
    start_index: usize,
    end_index: usize,
    link_type: LinkType,
    link_text: &'a str,
}

impl<'a> Link<'a> {
    fn from_capture(cap: Captures<'a>) -> Option<Link<'a>> {
        let link_type = match (cap.get(0), cap.get(1), cap.get(2)) {
            (_, Some(typ), Some(rest)) => {
                let mut path_props = rest.as_str().split_whitespace();
                let file_arg = path_props.next();

                match (typ.as_str(), file_arg) {
                    ("include", Some(pth)) => Some(parse_include_path(pth)),
                    _ => None,
                }
            }
            (Some(mat), None, None) if mat.as_str().starts_with(ESCAPE_CHAR) => {
                Some(LinkType::Escaped)
            }
            _ => None,
        };

        link_type.and_then(|lnk_type| {
            cap.get(0).map(|mat| Link {
                start_index: mat.start(),
                end_index: mat.end(),
                link_type: lnk_type,
                link_text: mat.as_str(),
            })
        })
    }

    fn render_with_path<P: AsRef<Path>>(&self, base: P) -> Result<String> {
        let base = base.as_ref();
        match self.link_type {
            // omit the escape char
            LinkType::Escaped => Ok(self.link_text[1..].to_owned()),
            LinkType::Include(ref pat, ref range_or_anchor) => {
                let target = base.join(pat);

                fs::read_to_string(&target)
                    .map(|s| match range_or_anchor {
                        RangeOrAnchor::Range(range) => take_lines(&s, range.clone()),
                        RangeOrAnchor::Anchor(anchor) => take_anchored_lines(&s, anchor),
                    })
                    .with_context(|| {
                        format!(
                            "Could not read file for link {} ({})",
                            self.link_text,
                            target.display(),
                        )
                    })
            }
        }
    }
}

struct LinkIter<'a>(CaptureMatches<'a, 'a>);

impl<'a> Iterator for LinkIter<'a> {
    type Item = Link<'a>;
    fn next(&mut self) -> Option<Link<'a>> {
        for cap in &mut self.0 {
            if let Some(inc) = Link::from_capture(cap) {
                return Some(inc);
            }
        }
        None
    }
}

fn find_links(contents: &str) -> LinkIter<'_> {
    // lazily compute following regex
    // r"\\\{\{#.*\}\}|\{\{#([a-zA-Z0-9]+)\s*([^}]+)\}\}")?;
    static RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(
            r"(?x)              # insignificant whitespace mode
        \\\{\{\#.*\}\}      # match escaped link
        |                   # or
        \{\{\s*             # link opening parens and whitespace
        \#([a-zA-Z0-9_]+)   # link type
        \s+                 # separating whitespace
        ([^}]+)             # link target path and space separated properties
        \}\}                # link closing parens",
        )
        .unwrap()
    });

    LinkIter(RE.captures_iter(contents))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_replace_all_escaped() {
        let start = r"
        Some text over here.
        ```hbs
        \{{#include file.rs}} << an escaped link!
        ```";
        let end = r"
        Some text over here.
        ```hbs
        {{#include file.rs}} << an escaped link!
        ```";
        assert_eq!(replace_all(start, "", "", 0), end);
    }

    #[test]
    fn test_find_links_no_link() {
        let s = "Some random text without link...";
        assert!(find_links(s).collect::<Vec<_>>() == vec![]);
    }

    #[test]
    fn test_find_links_partial_link() {
        let s = "Some random text with {{#playground...";
        assert!(find_links(s).collect::<Vec<_>>() == vec![]);
        let s = "Some random text with {{#include...";
        assert!(find_links(s).collect::<Vec<_>>() == vec![]);
        let s = "Some random text with \\{{#include...";
        assert!(find_links(s).collect::<Vec<_>>() == vec![]);
    }

    #[test]
    fn test_find_links_empty_link() {
        let s = "Some random text with {{#playground}} and {{#playground   }} {{}} {{#}}...";
        assert!(find_links(s).collect::<Vec<_>>() == vec![]);
    }

    #[test]
    fn test_find_links_unknown_link_type() {
        let s = "Some random text with {{#playgroundz ar.rs}} and {{#incn}} {{baz}} {{#bar}}...";
        assert!(find_links(s).collect::<Vec<_>>() == vec![]);
    }

    #[test]
    fn test_find_links_with_range() {
        let s = "Some random text with {{#include file.rs:10:20}}...";
        let res = find_links(s).collect::<Vec<_>>();
        println!("\nOUTPUT: {:?}\n", res);
        assert_eq!(
            res,
            vec![Link {
                start_index: 22,
                end_index: 48,
                link_type: LinkType::Include(
                    PathBuf::from("file.rs"),
                    RangeOrAnchor::Range(LineRange::from(9..20))
                ),
                link_text: "{{#include file.rs:10:20}}",
            }]
        );
    }

    #[test]
    fn test_find_links_with_line_number() {
        let s = "Some random text with {{#include file.rs:10}}...";
        let res = find_links(s).collect::<Vec<_>>();
        println!("\nOUTPUT: {:?}\n", res);
        assert_eq!(
            res,
            vec![Link {
                start_index: 22,
                end_index: 45,
                link_type: LinkType::Include(
                    PathBuf::from("file.rs"),
                    RangeOrAnchor::Range(LineRange::from(9..10))
                ),
                link_text: "{{#include file.rs:10}}",
            }]
        );
    }

    #[test]
    fn test_find_links_with_from_range() {
        let s = "Some random text with {{#include file.rs:10:}}...";
        let res = find_links(s).collect::<Vec<_>>();
        println!("\nOUTPUT: {:?}\n", res);
        assert_eq!(
            res,
            vec![Link {
                start_index: 22,
                end_index: 46,
                link_type: LinkType::Include(
                    PathBuf::from("file.rs"),
                    RangeOrAnchor::Range(LineRange::from(9..))
                ),
                link_text: "{{#include file.rs:10:}}",
            }]
        );
    }

    #[test]
    fn test_find_links_with_to_range() {
        let s = "Some random text with {{#include file.rs::20}}...";
        let res = find_links(s).collect::<Vec<_>>();
        println!("\nOUTPUT: {:?}\n", res);
        assert_eq!(
            res,
            vec![Link {
                start_index: 22,
                end_index: 46,
                link_type: LinkType::Include(
                    PathBuf::from("file.rs"),
                    RangeOrAnchor::Range(LineRange::from(..20))
                ),
                link_text: "{{#include file.rs::20}}",
            }]
        );
    }

    #[test]
    fn test_find_links_with_full_range() {
        let s = "Some random text with {{#include file.rs::}}...";
        let res = find_links(s).collect::<Vec<_>>();
        println!("\nOUTPUT: {:?}\n", res);
        assert_eq!(
            res,
            vec![Link {
                start_index: 22,
                end_index: 44,
                link_type: LinkType::Include(
                    PathBuf::from("file.rs"),
                    RangeOrAnchor::Range(LineRange::from(..))
                ),
                link_text: "{{#include file.rs::}}",
            }]
        );
    }

    #[test]
    fn test_find_links_with_no_range_specified() {
        let s = "Some random text with {{#include file.rs}}...";
        let res = find_links(s).collect::<Vec<_>>();
        println!("\nOUTPUT: {:?}\n", res);
        assert_eq!(
            res,
            vec![Link {
                start_index: 22,
                end_index: 42,
                link_type: LinkType::Include(
                    PathBuf::from("file.rs"),
                    RangeOrAnchor::Range(LineRange::from(..))
                ),
                link_text: "{{#include file.rs}}",
            }]
        );
    }

    #[test]
    fn test_find_links_with_anchor() {
        let s = "Some random text with {{#include file.rs:anchor}}...";
        let res = find_links(s).collect::<Vec<_>>();
        println!("\nOUTPUT: {:?}\n", res);
        assert_eq!(
            res,
            vec![Link {
                start_index: 22,
                end_index: 49,
                link_type: LinkType::Include(
                    PathBuf::from("file.rs"),
                    RangeOrAnchor::Anchor(String::from("anchor"))
                ),
                link_text: "{{#include file.rs:anchor}}",
            }]
        );
    }

    #[test]
    fn test_find_links_escaped_link() {
        let s = "Some random text with escaped playground \\{{#playground file.rs editable}} ...";

        let res = find_links(s).collect::<Vec<_>>();
        println!("\nOUTPUT: {:?}\n", res);

        assert_eq!(
            res,
            vec![Link {
                start_index: 41,
                end_index: 74,
                link_type: LinkType::Escaped,
                link_text: "\\{{#playground file.rs editable}}",
            }]
        );
    }

    #[test]
    fn parse_without_colon_includes_all() {
        let link_type = parse_include_path("arbitrary");
        assert_eq!(
            link_type,
            LinkType::Include(
                PathBuf::from("arbitrary"),
                RangeOrAnchor::Range(LineRange::from(RangeFull))
            )
        );
    }

    #[test]
    fn parse_with_nothing_after_colon_includes_all() {
        let link_type = parse_include_path("arbitrary:");
        assert_eq!(
            link_type,
            LinkType::Include(
                PathBuf::from("arbitrary"),
                RangeOrAnchor::Range(LineRange::from(RangeFull))
            )
        );
    }

    #[test]
    fn parse_with_two_colons_includes_all() {
        let link_type = parse_include_path("arbitrary::");
        assert_eq!(
            link_type,
            LinkType::Include(
                PathBuf::from("arbitrary"),
                RangeOrAnchor::Range(LineRange::from(RangeFull))
            )
        );
    }

    #[test]
    fn parse_with_garbage_after_two_colons_includes_all() {
        let link_type = parse_include_path("arbitrary::NaN");
        assert_eq!(
            link_type,
            LinkType::Include(
                PathBuf::from("arbitrary"),
                RangeOrAnchor::Range(LineRange::from(RangeFull))
            )
        );
    }

    #[test]
    fn parse_with_one_number_after_colon_only_that_line() {
        let link_type = parse_include_path("arbitrary:5");
        assert_eq!(
            link_type,
            LinkType::Include(
                PathBuf::from("arbitrary"),
                RangeOrAnchor::Range(LineRange::from(4..5))
            )
        );
    }

    #[test]
    fn parse_with_one_based_start_becomes_zero_based() {
        let link_type = parse_include_path("arbitrary:1");
        assert_eq!(
            link_type,
            LinkType::Include(
                PathBuf::from("arbitrary"),
                RangeOrAnchor::Range(LineRange::from(0..1))
            )
        );
    }

    #[test]
    fn parse_with_zero_based_start_stays_zero_based_but_is_probably_an_error() {
        let link_type = parse_include_path("arbitrary:0");
        assert_eq!(
            link_type,
            LinkType::Include(
                PathBuf::from("arbitrary"),
                RangeOrAnchor::Range(LineRange::from(0..1))
            )
        );
    }

    #[test]
    fn parse_start_only_range() {
        let link_type = parse_include_path("arbitrary:5:");
        assert_eq!(
            link_type,
            LinkType::Include(
                PathBuf::from("arbitrary"),
                RangeOrAnchor::Range(LineRange::from(4..))
            )
        );
    }

    #[test]
    fn parse_start_with_garbage_interpreted_as_start_only_range() {
        let link_type = parse_include_path("arbitrary:5:NaN");
        assert_eq!(
            link_type,
            LinkType::Include(
                PathBuf::from("arbitrary"),
                RangeOrAnchor::Range(LineRange::from(4..))
            )
        );
    }

    #[test]
    fn parse_end_only_range() {
        let link_type = parse_include_path("arbitrary::5");
        assert_eq!(
            link_type,
            LinkType::Include(
                PathBuf::from("arbitrary"),
                RangeOrAnchor::Range(LineRange::from(..5))
            )
        );
    }

    #[test]
    fn parse_start_and_end_range() {
        let link_type = parse_include_path("arbitrary:5:10");
        assert_eq!(
            link_type,
            LinkType::Include(
                PathBuf::from("arbitrary"),
                RangeOrAnchor::Range(LineRange::from(4..10))
            )
        );
    }

    #[test]
    fn parse_with_negative_interpreted_as_anchor() {
        let link_type = parse_include_path("arbitrary:-5");
        assert_eq!(
            link_type,
            LinkType::Include(
                PathBuf::from("arbitrary"),
                RangeOrAnchor::Anchor("-5".to_string())
            )
        );
    }

    #[test]
    fn parse_with_floating_point_interpreted_as_anchor() {
        let link_type = parse_include_path("arbitrary:-5.7");
        assert_eq!(
            link_type,
            LinkType::Include(
                PathBuf::from("arbitrary"),
                RangeOrAnchor::Anchor("-5.7".to_string())
            )
        );
    }

    #[test]
    fn parse_with_anchor_followed_by_colon() {
        let link_type = parse_include_path("arbitrary:some-anchor:this-gets-ignored");
        assert_eq!(
            link_type,
            LinkType::Include(
                PathBuf::from("arbitrary"),
                RangeOrAnchor::Anchor("some-anchor".to_string())
            )
        );
    }

    #[test]
    fn parse_with_more_than_three_colons_ignores_everything_after_third_colon() {
        let link_type = parse_include_path("arbitrary:5:10:17:anything:");
        assert_eq!(
            link_type,
            LinkType::Include(
                PathBuf::from("arbitrary"),
                RangeOrAnchor::Range(LineRange::from(4..10))
            )
        );
    }
}
