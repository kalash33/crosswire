//! `serde` adapters for byte payloads.
//!
//! Wire choice (ruled, part of the frozen schema): with the `std` feature
//! (the default) byte payloads serialise as a standard-alphabet padded
//! base64 string, so JSON in project files and the API stays compact and
//! readable. Without `std` they serialise as a plain byte sequence
//! (`serialize_bytes`; self-describing formats read it back as a sequence
//! of integers). Deserialisation under `std` accepts only base64 strings.
//! Both ends of a stored or transmitted document must therefore agree on
//! the feature; the project file and API always use `std`.
//!
//! Used with `#[serde(with = "...")]` on [`Bytes`](bytes::Bytes) fields
//! and, via the `hash` submodule, on `[u8; 32]` content hashes.

use alloc::vec::Vec;
use bytes::Bytes;
use serde::{Deserializer, Serializer};

/// Serialises raw bytes per the module's wire choice.
fn ser_bytes<S: Serializer>(b: &[u8], s: S) -> Result<S::Ok, S::Error> {
    #[cfg(feature = "std")]
    {
        s.serialize_str(&crate::b64::encode(b))
    }
    #[cfg(not(feature = "std"))]
    {
        s.serialize_bytes(b)
    }
}

/// Deserialises raw bytes per the module's wire choice.
fn de_bytes<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
    use core::fmt;
    use serde::de::{self, Visitor};

    struct V;
    impl<'de> Visitor<'de> for V {
        type Value = Vec<u8>;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            #[cfg(feature = "std")]
            {
                f.write_str("a base64 string")
            }
            #[cfg(not(feature = "std"))]
            {
                f.write_str("a byte sequence")
            }
        }

        #[cfg(feature = "std")]
        fn visit_str<E: de::Error>(self, v: &str) -> Result<Vec<u8>, E> {
            crate::b64::decode(v).ok_or_else(|| E::custom("invalid base64"))
        }

        #[cfg(not(feature = "std"))]
        fn visit_bytes<E: de::Error>(self, v: &[u8]) -> Result<Vec<u8>, E> {
            Ok(v.to_vec())
        }

        #[cfg(not(feature = "std"))]
        fn visit_byte_buf<E: de::Error>(self, v: Vec<u8>) -> Result<Vec<u8>, E> {
            Ok(v)
        }

        #[cfg(not(feature = "std"))]
        fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut a: A) -> Result<Vec<u8>, A::Error> {
            let mut out = Vec::new();
            while let Some(b) = a.next_element::<u8>()? {
                out.push(b);
            }
            Ok(out)
        }
    }

    #[cfg(feature = "std")]
    {
        d.deserialize_str(V)
    }
    #[cfg(not(feature = "std"))]
    {
        d.deserialize_byte_buf(V)
    }
}

/// `#[serde(with)]` for [`Bytes`].
pub(crate) fn serialize<S: Serializer>(b: &Bytes, s: S) -> Result<S::Ok, S::Error> {
    ser_bytes(b, s)
}

/// `#[serde(with)]` for [`Bytes`].
pub(crate) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Bytes, D::Error> {
    de_bytes(d).map(Bytes::from)
}

/// `#[serde(with)]` for 32-byte content hashes.
pub(crate) mod hash {
    use super::{de_bytes, ser_bytes};
    use serde::{de::Error, Deserializer, Serializer};

    pub(crate) fn serialize<S: Serializer>(h: &[u8; 32], s: S) -> Result<S::Ok, S::Error> {
        ser_bytes(h, s)
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 32], D::Error> {
        let v = de_bytes(d)?;
        <[u8; 32]>::try_from(v.as_slice()).map_err(|_| D::Error::custom("hash must be 32 bytes"))
    }
}
