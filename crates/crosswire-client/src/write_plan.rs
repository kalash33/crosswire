//! What to write for one request.

use bytes::Bytes;

/// A point at which a write may pause. Reserved: no hold is supported yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum HoldPoint {
    /// Wait until released by the caller.
    Manual,
}

/// One piece of a [`WritePlan`], written as a single unit.
#[derive(Clone, Debug)]
pub struct Segment {
    /// The bytes to write.
    pub bytes: Bytes,
    /// Where to pause after writing them; `None` writes straight through.
    pub hold: Option<HoldPoint>,
}

impl Segment {
    /// A segment with no hold.
    #[must_use]
    pub fn new(bytes: Bytes) -> Segment {
        Segment { bytes, hold: None }
    }

    /// Sets the hold point.
    #[must_use]
    pub fn with_hold(mut self, hold: HoldPoint) -> Segment {
        self.hold = Some(hold);
        self
    }
}

/// The exact bytes of a request, in the order they are written.
#[derive(Clone, Debug, Default)]
pub struct WritePlan {
    segments: Vec<Segment>,
}

impl WritePlan {
    /// A plan that writes `bytes` in one segment.
    #[must_use]
    pub fn raw(bytes: Bytes) -> WritePlan {
        WritePlan {
            segments: vec![Segment::new(bytes)],
        }
    }

    /// A plan from explicit segments.
    #[must_use]
    pub fn from_segments(segments: Vec<Segment>) -> WritePlan {
        WritePlan { segments }
    }

    /// The segments, in write order.
    #[must_use]
    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }
}
