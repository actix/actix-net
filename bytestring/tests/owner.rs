//! Public API tests for owner operations.

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use bytestring::ByteString;

struct Owner {
    text: String,
    drops: Arc<AtomicUsize>,
}

impl AsRef<str> for Owner {
    fn as_ref(&self) -> &str {
        &self.text
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn owner_lives_until_last_shared_view_is_dropped() {
    let drops = Arc::new(AtomicUsize::new(0));
    let owner = Owner {
        text: String::from("aµz"),
        drops: drops.clone(),
    };
    let ptr = owner.text.as_ptr();
    let text = ByteString::from_owner(owner);
    assert_eq!(text.as_ptr(), ptr);
    let clone = text.clone();
    let part = text.slice_ref(&text[1..3]);
    drop(text);
    drop(clone);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    assert_eq!(part, "µ");
    drop(part);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[test]
fn empty_owner_is_released() {
    let drops = Arc::new(AtomicUsize::new(0));
    let text = ByteString::from_owner(Owner {
        text: String::new(),
        drops: drops.clone(),
    });
    assert!(text.is_empty());
    drop(text);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}
