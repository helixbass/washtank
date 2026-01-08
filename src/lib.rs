use std::fs::OpenOptions;
use std::io::Write;

mod config;
pub mod editor;
mod fold;
mod indent;
mod jsonrpc;
mod lsp;
pub mod tree_sitter;

pub use config::{Args, Config, InitialFile};
pub use editor::{Editor, EventAggregator, PrintedLine};
pub use fold::{calculate_folds, Fold, FoldIndex};
pub use indent::{calculate_indents, IndentLevel};
pub use jsonrpc::{RequestMessage, RpcMessage};
pub use lsp::{run_rust_analyzer, LspIncomingMessage, LspOutgoingMessage};
pub use tree_sitter::TreeSitterHighlight;

pub type LineNumber = usize;

pub fn strip_trailing_newline(file_contents: &str) -> &str {
    if file_contents.ends_with("\n") {
        &file_contents[..file_contents.len() - 1]
    } else {
        file_contents
    }
}

#[allow(dead_code)]
pub fn log(str: &str) {
    let mut file = OpenOptions::new()
        .append(true)
        .create(true)
        .open("dev.log")
        .unwrap();

    writeln!(file, "{str}").unwrap();
}
