//! Public API tests for range slicing.

use std::ops::Bound;

use bytestring::ByteString;

#[test]
fn ranges_share_storage() {
    let text = ByteString::from("aµz");
    let part = text.slice(1..3);
    assert_eq!(part, "µ");
    assert_eq!(part.as_ptr(), text[1..].as_ptr());
    assert_eq!(text.slice(..), text);
    assert_eq!(text.slice(..=2), "aµ");
    assert_eq!(text.slice(3..), "z");
    assert_eq!(text.slice(4..4), "");
    assert_eq!(text.slice((Bound::Excluded(0), Bound::Excluded(3))), "µ");
    drop(text);
    assert_eq!(part, "µ");
}

#[test]
fn invalid_ranges_panic() {
    let text = ByteString::from("aµz");
    for (start, end) in [(2, 3), (0, 2), (3, 1), (0, 5)] {
        assert!(std::panic::catch_unwind(|| text.slice(start..end)).is_err());
    }
    assert!(std::panic::catch_unwind(|| text.slice(..=usize::MAX)).is_err());
    assert!(std::panic::catch_unwind(|| {
        text.slice((Bound::Excluded(usize::MAX), Bound::Unbounded))
    })
    .is_err());
}
