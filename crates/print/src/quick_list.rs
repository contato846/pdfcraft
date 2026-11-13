//! The printer list without the wait. Listing the queues is quick; asking each driver what it
//! can do (two-sided) is not, and for a network printer that is switched off or out of reach it
//! can take minutes. Read one after another before the list is shown, a single unreachable
//! printer kept every printer from being offered. Here each driver is asked on a thread of its
//! own and the list is returned after a short wait with whatever has answered; a late answer is
//! kept for the next time the list is read.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, PoisonError, mpsc};
use std::time::{Duration, Instant};

/// What is known about the drivers so far, and which are still being asked.
#[derive(Default)]
pub(crate) struct Answers {
    known: BTreeMap<String, bool>,
    asking: BTreeSet<String>,
}

/// For each named item, `read`'s answer if it comes within `wait` (or is already known), else
/// `None`. An item still being asked from an earlier call is not asked again. Order is kept.
pub(crate) fn answers_within<T: Send + 'static>(
    items: Vec<(String, T)>,
    read: fn(&T) -> Option<bool>,
    wait: Duration,
    answers: &'static Mutex<Answers>,
) -> Vec<(String, Option<bool>)> {
    let (tx, rx) = mpsc::channel::<(String, Option<bool>)>();
    let mut out: Vec<(String, Option<bool>)> = Vec::with_capacity(items.len());
    let mut pending = 0usize;
    for (name, item) in items {
        let mut state = answers.lock().unwrap_or_else(PoisonError::into_inner);
        let known = state.known.get(&name).copied();
        out.push((name.clone(), known));
        if known.is_some() || !state.asking.insert(name.clone()) {
            continue;
        }
        drop(state);
        let tx = tx.clone();
        let asked = name.clone();
        let spawned = std::thread::Builder::new().name("printer capabilities".into()).spawn(move || {
            let answer = read(&item);
            let mut state = answers.lock().unwrap_or_else(PoisonError::into_inner);
            state.asking.remove(&asked);
            if let Some(a) = answer {
                state.known.insert(asked.clone(), a);
            }
            drop(state);
            // Nobody may be waiting any more: the answer is kept above either way.
            let _ = tx.send((asked, answer));
        });
        match spawned {
            Ok(_) => pending += 1,
            Err(_) => {
                answers.lock().unwrap_or_else(PoisonError::into_inner).asking.remove(&name);
            }
        }
    }
    drop(tx);
    let deadline = Instant::now() + wait;
    while pending > 0 {
        let left = deadline.saturating_duration_since(Instant::now());
        let Ok((name, answer)) = rx.recv_timeout(left) else { break };
        pending -= 1;
        if let Some(slot) = out.iter_mut().find(|(n, _)| *n == name) {
            slot.1 = answer;
        }
    }
    out
}

/// The spooler's printers on Windows: local queues and the user's connections, the default
/// marked, sorted by name. Two-sided support is filled in for the drivers that answer in time.
#[cfg(windows)]
pub(crate) fn printers() -> Vec<craft_print::Printer> {
    use winprint::printer::PrinterDevice;
    use winprint::ticket::PrintCapabilities;
    use winprint::ticket::document::{NS_PSK, OwnedName};

    /// How long the list waits for the drivers. Local drivers answer well within it.
    const WAIT: Duration = Duration::from_millis(2500);
    static ANSWERS: Mutex<Answers> = Mutex::new(Answers { known: BTreeMap::new(), asking: BTreeSet::new() });

    fn two_sided(device: &PrinterDevice) -> Option<bool> {
        let capabilities = PrintCapabilities::fetch(device).ok()?;
        let duplex = OwnedName::qualified("JobDuplexAllDocumentsContiguously", NS_PSK, Some("psk"));
        Some(capabilities.options_for_feature(duplex).any(|o| o.name.as_ref().is_some_and(|n| n.local_name.starts_with("TwoSided"))))
    }

    let Ok(devices) = PrinterDevice::all() else { return Vec::new() };
    let default = winsafe::GetDefaultPrinter().ok().filter(|n| !n.is_empty());
    let mut items: Vec<(String, PrinterDevice)> =
        devices.into_iter().map(|d| (d.name().to_string(), d)).filter(|(n, _)| !n.trim().is_empty()).collect();
    items.sort_by(|a, b| a.0.cmp(&b.0));
    items.dedup_by(|a, b| a.0 == b.0);
    answers_within(items, two_sided, WAIT, &ANSWERS)
        .into_iter()
        .map(|(name, two_sided)| craft_print::Printer {
            default: default.as_deref() == Some(name.as_str()),
            name,
            description: String::new(),
            two_sided,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One driver that does not answer no longer holds the list: the others are listed with
    /// their answers at once, the silent one as unknown, and its late answer is there next time.
    #[test]
    fn a_silent_driver_does_not_hold_the_list() {
        static ANSWERS: Mutex<Answers> = Mutex::new(Answers { known: BTreeMap::new(), asking: BTreeSet::new() });
        fn read(ms: &u64) -> Option<bool> {
            std::thread::sleep(Duration::from_millis(*ms));
            Some(*ms != 0)
        }
        let items = || vec![("Local".to_string(), 0u64), ("Remote".to_string(), 1500u64)];
        let started = Instant::now();
        let first = answers_within(items(), read, Duration::from_millis(300), &ANSWERS);
        assert!(started.elapsed() < Duration::from_millis(1200), "the list did not wait for the silent driver");
        assert_eq!(first, vec![("Local".to_string(), Some(false)), ("Remote".to_string(), None)]);
        // Asked again while the first question is still open: no second thread, still unknown.
        let again = answers_within(items(), read, Duration::from_millis(50), &ANSWERS);
        assert_eq!(again[1], ("Remote".to_string(), None));
        std::thread::sleep(Duration::from_millis(1800));
        let later = answers_within(items(), read, Duration::from_millis(50), &ANSWERS);
        assert_eq!(later, vec![("Local".to_string(), Some(false)), ("Remote".to_string(), Some(true))]);
    }

    /// A driver that cannot say (an error) is asked again next time, not remembered as unknown.
    #[test]
    fn an_answer_that_failed_is_asked_again() {
        static ANSWERS: Mutex<Answers> = Mutex::new(Answers { known: BTreeMap::new(), asking: BTreeSet::new() });
        static CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        fn read(_: &()) -> Option<bool> {
            CALLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            None
        }
        for _ in 0..2 {
            assert_eq!(answers_within(vec![("P".to_string(), ())], read, Duration::from_secs(2), &ANSWERS), vec![("P".to_string(), None)]);
        }
        assert_eq!(CALLS.load(std::sync::atomic::Ordering::SeqCst), 2);
    }
}
