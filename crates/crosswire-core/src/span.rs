//! Byte spans into the original input.

/// A half-open byte range `[start, end)` into the original input.
///
/// `end` is exclusive. The invariant `start <= end` holds for every value,
/// including deserialized ones: deserializing `{"start": 5, "end": 2}` fails.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(try_from = "raw::RawSpan")
)]
pub struct Span {
    start: u32,
    end: u32,
}

impl Span {
    /// Creates a span. Panics in debug builds if `end < start`; in release
    /// builds `end` is clamped to `start`.
    #[must_use]
    pub fn new(start: u32, end: u32) -> Span {
        debug_assert!(end >= start, "Span end ({end}) < start ({start})");
        Span {
            start,
            end: end.max(start),
        }
    }

    /// Inclusive start offset.
    #[must_use]
    pub fn start(&self) -> u32 {
        self.start
    }

    /// Exclusive end offset.
    #[must_use]
    pub fn end(&self) -> u32 {
        self.end
    }

    /// Length in bytes.
    #[must_use]
    pub fn len(&self) -> u32 {
        self.end.saturating_sub(self.start)
    }

    /// Whether the span covers no bytes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }

    /// The span as a `usize` range, suitable for slicing.
    #[must_use]
    pub fn range(&self) -> core::ops::Range<usize> {
        self.start as usize..self.end as usize
    }
}

#[cfg(feature = "serde")]
mod raw {
    /// The unchecked wire form of a [`Span`](super::Span); converted with a
    /// check that `start <= end`.
    #[derive(serde::Deserialize)]
    pub struct RawSpan {
        start: u32,
        end: u32,
    }

    impl TryFrom<RawSpan> for super::Span {
        type Error = &'static str;

        fn try_from(r: RawSpan) -> Result<Self, Self::Error> {
            if r.end < r.start {
                return Err("span end is before its start");
            }
            Ok(super::Span {
                start: r.start,
                end: r.end,
            })
        }
    }
}
