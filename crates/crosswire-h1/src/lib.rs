#![doc = include_str!("../README.md")]
#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

mod chunked;
#[cfg(feature = "http")]
mod compat_http;
mod config;
mod error;
mod limits;
mod message;
mod parser;
mod progress;
mod writer;

pub use chunked::{BodyDecoder, BodyEvent};
#[cfg(feature = "http")]
pub use compat_http::HttpConvertError;
pub use config::{ParserConfig, ParserConfigBuilder};
pub use error::ParseError;
pub use limits::{ParseLimits, ParseLimitsBuilder};
pub use message::{
    BodyFraming, HeadView, HeaderSpan, Kind, Method, RawMessage, ResolutionRule, ResponseContext,
    StartLine,
};
pub use parser::Parser;
pub use progress::{Progress, Step};
pub use writer::{write_exact, BodyEncoder};
