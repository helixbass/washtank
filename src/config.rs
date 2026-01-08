use std::path::PathBuf;

use clap::Parser;
use derive_builder::Builder;

#[derive(Parser)]
pub struct Args {
    pub file_name: PathBuf,
}

#[derive(Builder)]
pub struct Config {
    pub initial_file: InitialFile,
    #[builder(setter(strip_option))]
    pub flex_grow: Option<f64>,
}

impl From<Args> for Config {
    fn from(value: Args) -> Self {
        ConfigBuilder::default()
            .initial_file(InitialFile::Path(value.file_name))
            .build()
            .unwrap()
    }
}

#[derive(Clone)]
pub enum InitialFile {
    Path(PathBuf),
    Anonymous(String),
}
