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

/// `#[serde(with)]` for a list of byte payloads, each encoded like any
/// other byte payload.
pub(crate) mod vec {
    use super::{de_bytes, ser_bytes};
    use alloc::vec::Vec;
    use bytes::Bytes;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    struct Ref<'a>(&'a [u8]);

    impl Serialize for Ref<'_> {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            ser_bytes(self.0, s)
        }
    }

    struct Own(Vec<u8>);

    impl<'de> Deserialize<'de> for Own {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Own, D::Error> {
            de_bytes(d).map(Own)
        }
    }

    pub(crate) fn serialize<S: Serializer>(v: &[Bytes], s: S) -> Result<S::Ok, S::Error> {
        s.collect_seq(v.iter().map(|b| Ref(b)))
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Bytes>, D::Error> {
        let v = Vec::<Own>::deserialize(d)?;
        Ok(v.into_iter().map(|o| Bytes::from(o.0)).collect())
    }
}

/// `#[serde(with)]` for ordered `(name, bytes)` pairs: each value is encoded
/// like any other byte payload.
pub(crate) mod pairs {
    use super::{de_bytes, ser_bytes};
    use alloc::string::String;
    use alloc::vec::Vec;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    struct Ref<'a>(&'a [u8]);

    impl Serialize for Ref<'_> {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            ser_bytes(self.0, s)
        }
    }

    struct Own(Vec<u8>);

    impl<'de> Deserialize<'de> for Own {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Own, D::Error> {
            de_bytes(d).map(Own)
        }
    }

    pub(crate) fn serialize<S: Serializer>(
        map: &[(String, Vec<u8>)],
        s: S,
    ) -> Result<S::Ok, S::Error> {
        s.collect_seq(map.iter().map(|(n, v)| (n.as_str(), Ref(v))))
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<Vec<(String, Vec<u8>)>, D::Error> {
        let v = Vec::<(String, Own)>::deserialize(d)?;
        Ok(v.into_iter().map(|(n, o)| (n, o.0)).collect())
    }
}
