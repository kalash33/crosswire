//! `serde` adapters for [`Ulid`].
//!
//! `ulid`'s own `serde` feature does not compile without `std`, so ids are
//! (de)serialised here as their canonical 26-character Crockford base32
//! string, the same wire form that feature produces.

use alloc::vec::Vec;
use core::fmt;
use serde::de::{Error, SeqAccess, Visitor};
use serde::{Deserializer, Serializer};
use ulid::Ulid;

/// `#[serde(with)]` for a plain `Ulid`.
pub(crate) fn serialize<S: Serializer>(id: &Ulid, s: S) -> Result<S::Ok, S::Error> {
    let mut buf = [0u8; ulid::ULID_LEN];
    s.serialize_str(id.array_to_str(&mut buf))
}

struct IdVisitor;

impl Visitor<'_> for IdVisitor {
    type Value = Ulid;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a 26-character ULID string")
    }

    fn visit_str<E: Error>(self, v: &str) -> Result<Ulid, E> {
        Ulid::from_string(v).map_err(E::custom)
    }
}

/// `#[serde(with)]` for a plain `Ulid`.
pub(crate) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Ulid, D::Error> {
    d.deserialize_str(IdVisitor)
}

/// `#[serde(with)]` for `Option<Ulid>`.
pub(crate) mod option {
    use super::{IdVisitor, Ulid};
    use core::fmt;
    use serde::{Deserializer, Serializer};

    pub(crate) fn serialize<S: Serializer>(id: &Option<Ulid>, s: S) -> Result<S::Ok, S::Error> {
        match id {
            Some(id) => {
                let mut buf = [0u8; ulid::ULID_LEN];
                s.serialize_some(&*id.array_to_str(&mut buf))
            }
            None => s.serialize_none(),
        }
    }

    struct OptVisitor;

    impl<'de> serde::de::Visitor<'de> for OptVisitor {
        type Value = Option<Ulid>;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("null or a ULID string")
        }

        fn visit_none<E: serde::de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
            d.deserialize_str(IdVisitor).map(Some)
        }
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Ulid>, D::Error> {
        d.deserialize_option(OptVisitor)
    }
}

/// `#[serde(with)]` for `Vec<Ulid>`.
pub(crate) mod vec {
    use super::{SeqAccess, Ulid, Vec, Visitor};
    use core::fmt;
    use serde::ser::SerializeSeq;
    use serde::{Deserialize, Deserializer, Serializer};

    struct Id(Ulid);

    impl<'de> Deserialize<'de> for Id {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Id, D::Error> {
            super::deserialize(d).map(Id)
        }
    }

    pub(crate) fn serialize<S: Serializer>(ids: &[Ulid], s: S) -> Result<S::Ok, S::Error> {
        let mut seq = s.serialize_seq(Some(ids.len()))?;
        for id in ids {
            let mut buf = [0u8; ulid::ULID_LEN];
            seq.serialize_element(&*id.array_to_str(&mut buf))?;
        }
        seq.end()
    }

    struct SeqVisitor;

    impl<'de> Visitor<'de> for SeqVisitor {
        type Value = Vec<Ulid>;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("a list of ULID strings")
        }

        fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> Result<Vec<Ulid>, A::Error> {
            let mut out = Vec::new();
            while let Some(Id(id)) = a.next_element()? {
                out.push(id);
            }
            Ok(out)
        }
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Ulid>, D::Error> {
        d.deserialize_seq(SeqVisitor)
    }
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};
    use ulid::Ulid;

    #[derive(Serialize, Deserialize, PartialEq, Debug)]
    struct W {
        #[serde(with = "super")]
        a: Ulid,
        #[serde(default, with = "super::option")]
        b: Option<Ulid>,
        #[serde(default, with = "super::vec")]
        c: alloc::vec::Vec<Ulid>,
    }

    #[test]
    fn roundtrip_as_canonical_strings() {
        let id = Ulid::from_parts(1_700_000_000_000, 7);
        let w = W {
            a: id,
            b: Some(id),
            c: alloc::vec![id, id],
        };
        let s = serde_json::to_string(&w).unwrap();
        assert!(s.contains(&alloc::format!("\"a\":\"{id}\"")), "{s}");
        assert_eq!(serde_json::from_str::<W>(&s).unwrap(), w);
        let none: W =
            serde_json::from_str(&alloc::format!("{{\"a\":\"{id}\",\"b\":null}}")).unwrap();
        assert_eq!((none.b, none.c.len()), (None, 0));
        assert!(serde_json::from_str::<W>("{\"a\":\"nope\"}").is_err());
    }
}
