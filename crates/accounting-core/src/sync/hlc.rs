//! Hybrid logical clocks: an ordering two devices can agree on.
//!
//! # Why not `updated_at`
//!
//! Every row already carries one, and it is the obvious thing to resolve a
//! conflict with: newest edit wins. It is also wrong, because `updated_at` is
//! each device reading *its own* clock. A phone three minutes fast makes every
//! edit it has ever made look newer than a laptop edit that genuinely came
//! afterwards, so the laptop's correction to a receipt loses to the phone's
//! original and there is nothing on either screen to say so. Wall clocks on
//! consumer devices disagree by seconds routinely and by minutes often; the
//! failure is silent and it corrupts the newest data first.
//!
//! # What an HLC adds
//!
//! Three parts, compared in order:
//!
//! * `millis` — wall clock, but never allowed to go backwards, and dragged
//!   forward by any *later* timestamp this device has seen. Two devices that
//!   have exchanged a bundle therefore share a floor, and the skew stops
//!   mattering the moment they talk.
//! * `counter` — breaks ties inside one millisecond, which is not rare: saving
//!   a receipt writes the transaction and its splits in one SQLite
//!   transaction, well under a millisecond apart.
//! * `device` — breaks the remaining ties so the order is *total*. Arbitrary
//!   but stable: both devices pick the same winner, which is the only property
//!   that matters. A rule that leaves two edits incomparable leaves them to be
//!   resolved differently on each side, which is how replicas silently diverge.
//!
//! # Reading the clock here is allowed
//!
//! The crate forbids date logic derived from the clock (rule 3), because it is
//! untestable and drags `chrono`'s timezone lookup — JNI on Android — into the
//! core. This is neither: it is `SystemTime` since the epoch, the same
//! justification [`crate::db::new_uuid`] already relies on for uuid v7. Every
//! function that advances a clock takes the reading as an argument, so the
//! tests below are exact rather than approximate.

use std::fmt;

use serde::{Deserialize, Serialize};

/// A device's identity within one sync group.
///
/// Not the uuid of any row: it names the *replica*, and it outlives every row
/// on it. Minted once per database, stored in `app_setting`.
pub type DeviceId = String;

/// One point on the shared timeline.
///
/// Ordered by `(millis, counter, device)`, and that order is the whole point:
/// it is total, it respects causality, and it collapses to plain wall-clock
/// time whenever the two clocks agree.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize, Deserialize)]
pub struct Hlc {
    /// Milliseconds since the Unix epoch, monotonically non-decreasing for
    /// this device.
    pub millis: u64,
    /// Disambiguates writes inside one millisecond.
    pub counter: u32,
    /// Breaks the last tie, so the order is total.
    pub device: DeviceId,
}

impl Hlc {
    /// The clock a brand-new database starts from.
    #[must_use]
    pub fn zero(device: DeviceId) -> Self {
        Hlc {
            millis: 0,
            counter: 0,
            device,
        }
    }

    /// The next local timestamp, given the wall clock now.
    ///
    /// `now` going backwards — a clock correction, a timezone change, a laptop
    /// waking up — must not produce a timestamp that sorts before one already
    /// issued, or an edit made later would lose to one made earlier on the same
    /// device. So the clock holds still and the counter advances instead.
    #[must_use]
    pub fn tick(&self, now_millis: u64) -> Self {
        if now_millis > self.millis {
            Hlc {
                millis: now_millis,
                counter: 0,
                device: self.device.clone(),
            }
        } else {
            Hlc {
                millis: self.millis,
                counter: self.counter.saturating_add(1),
                device: self.device.clone(),
            }
        }
    }

    /// Fold in a timestamp that arrived from another device.
    ///
    /// This is what makes skew stop mattering. After receiving a bundle, this
    /// device's clock is at least as late as anything in it, so every edit it
    /// makes from now on sorts *after* everything it has seen — which is
    /// exactly the causality a plain wall clock cannot express.
    #[must_use]
    pub fn receive(&self, remote: &Hlc, now_millis: u64) -> Self {
        let millis = now_millis.max(self.millis).max(remote.millis);
        let counter = if millis == self.millis && millis == remote.millis {
            self.counter.max(remote.counter).saturating_add(1)
        } else if millis == self.millis {
            self.counter.saturating_add(1)
        } else if millis == remote.millis {
            remote.counter.saturating_add(1)
        } else {
            0
        };
        Hlc {
            millis,
            counter,
            device: self.device.clone(),
        }
    }
}

/// `"00000198f1a2b3c4-0000002a-laptop"`.
///
/// Fixed-width hex for the two numbers so that **string** order matches
/// numeric order. That is not decoration: these are stored in SQLite `TEXT`
/// columns and compared with `<` in SQL, where `'9' > '10'` would otherwise put
/// a later edit before an earlier one on the ten-thousandth write.
impl fmt::Display for Hlc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}-{:08x}-{}", self.millis, self.counter, self.device)
    }
}

/// The error from parsing a malformed timestamp.
#[derive(Debug, thiserror::Error)]
#[error("{0:?} is not a hybrid logical clock")]
pub struct ParseHlcError(String);

impl std::str::FromStr for Hlc {
    type Err = ParseHlcError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let bad = || ParseHlcError(text.to_string());
        // `splitn`, not `split`: a device id may contain dashes, and only the
        // first two fields are fixed.
        let mut parts = text.splitn(3, '-');
        let millis = parts.next().ok_or_else(bad)?;
        let counter = parts.next().ok_or_else(bad)?;
        let device = parts.next().ok_or_else(bad)?;
        if millis.len() != 16 || counter.len() != 8 || device.is_empty() {
            return Err(bad());
        }
        Ok(Hlc {
            millis: u64::from_str_radix(millis, 16).map_err(|_| bad())?,
            counter: u32::from_str_radix(counter, 16).map_err(|_| bad())?,
            device: device.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hlc(millis: u64, counter: u32, device: &str) -> Hlc {
        Hlc {
            millis,
            counter,
            device: device.to_string(),
        }
    }

    #[test]
    fn a_later_wall_clock_resets_the_counter() {
        let clock = hlc(1_000, 7, "laptop");
        assert_eq!(clock.tick(2_000), hlc(2_000, 0, "laptop"));
    }

    #[test]
    fn writes_inside_one_millisecond_still_order() {
        // Saving a receipt writes the transaction and its splits in one SQLite
        // transaction, well under a millisecond apart.
        let first = hlc(1_000, 0, "laptop").tick(1_000);
        let second = first.tick(1_000);
        assert!(second > first);
        assert_eq!(second.counter, 2);
    }

    #[test]
    fn a_clock_that_goes_backwards_does_not_reorder_history() {
        // A clock correction, a timezone change, a laptop waking up. The edit
        // made later must not sort before the edit made earlier.
        let earlier = hlc(5_000, 0, "laptop");
        let later = earlier.tick(4_000);
        assert!(later > earlier, "an edit made later must sort later");
        assert_eq!(later.millis, 5_000, "the clock holds rather than rewinds");
    }

    #[test]
    fn receiving_drags_the_clock_past_anything_it_has_seen() {
        // The property that makes skew stop mattering: after a bundle lands,
        // every local edit sorts after everything in it.
        let local = hlc(1_000, 0, "phone");
        let remote = hlc(9_000, 3, "laptop");
        let merged = local.receive(&remote, 1_100);
        assert!(merged > remote);
        assert_eq!(merged.device, "phone", "the clock stays ours");

        let next = merged.tick(1_200);
        assert!(next > remote, "and so does everything after it");
    }

    #[test]
    fn the_order_is_total_even_for_simultaneous_edits() {
        // Two devices, same millisecond, same counter. Neither is "right", but
        // both must pick the *same* winner or the replicas diverge.
        let a = hlc(1_000, 0, "aaa");
        let b = hlc(1_000, 0, "bbb");
        assert_ne!(a.cmp(&b), std::cmp::Ordering::Equal);
        assert_eq!(a.cmp(&b), "aaa".cmp("bbb"));
    }

    #[test]
    fn text_order_matches_clock_order() {
        // These are compared with `<` in SQL, where '9' > '10'. Fixed-width hex
        // is what stops a later edit sorting before an earlier one.
        let mut clocks = vec![
            hlc(9, 0, "d"),
            hlc(10, 0, "d"),
            hlc(9, 10, "d"),
            hlc(9, 9, "d"),
        ];
        clocks.sort();
        let mut as_text: Vec<String> = clocks.iter().map(ToString::to_string).collect();
        let sorted_by_value = as_text.clone();
        as_text.sort();
        assert_eq!(as_text, sorted_by_value);
    }

    #[test]
    fn round_trips_through_text_including_a_device_with_dashes() {
        for device in ["laptop", "tanner-phone", "a-b-c"] {
            let clock = hlc(0x0198_f1a2_b3c4, 42, device);
            let text = clock.to_string();
            assert_eq!(text.parse::<Hlc>().expect("parses"), clock, "{text}");
        }
    }

    #[test]
    fn malformed_text_is_refused_rather_than_guessed_at() {
        for bad in ["", "nope", "0198f1a2b3c4-2a-laptop", "00000198f1a2b3c4-0000002a-"] {
            assert!(bad.parse::<Hlc>().is_err(), "{bad:?} should not parse");
        }
    }
}
