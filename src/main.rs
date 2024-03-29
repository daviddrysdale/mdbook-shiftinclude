//! mdBook preprocessor for file inclusion with shift
use clap::{App, Arg, SubCommand};
use log::warn;
use mdbook::{
    book::{Book, BookItem},
    errors::Error,
    preprocess::{CmdPreprocessor, Preprocessor, PreprocessorContext},
};
use std::{io, process};

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

    fn run(&self, _ctx: &PreprocessorContext, mut book: Book) -> Result<Book, Error> {
        book.for_each_mut(|section: &mut BookItem| {
            if let BookItem::Chapter(ch) = section {
                if let Some(_ch_path) = &ch.path {
                    // TODO
                }
            }
        });
        Ok(book)
    }
}
