use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Lsp: {0}")]
    Lsp(String),
}
