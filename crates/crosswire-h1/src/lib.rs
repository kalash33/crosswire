//! HTTP/1.1 message parsing and serialization for Crosswire.
#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

mod chunked;
mod config;
mod error;
mod limits;
mod message;
mod parser;
mod progress;

pub use chunked::{BodyDecoder, BodyEvent};
pub use config::{ParserConfig, ParserConfigBuilder};
pub use error::ParseError;
pub use limits::{ParseLimits, ParseLimitsBuilder};
pub use message::{
    BodyFraming, HeadView, HeaderSpan, Kind, Method, RawMessage, ResolutionRule, ResponseContext,
    StartLine,
};
pub use parser::Parser;
pub use progress::{Progress, Step};
