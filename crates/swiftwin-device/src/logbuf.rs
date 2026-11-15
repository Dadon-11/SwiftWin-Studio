//! Ring buffer of log entries with blocking "follow" support.
//!
//! DEBUG entries get a ring of their own. A real iPhone logs about 2,600 lines a second, almost
//! all DEBUG, and every WDA UI query makes the app under test log thousands of accessibility DEBUG
//! lines: in one shared ring an app's own notices were pushed out within seconds (seen on an
//! iPhone with iOS 27.0.1). With separate rings the other levels (~150 lines/s) keep minutes.

use std::collections::VecDeque;
use std::sync::{Condvar, Mutex};
use std::time::Duration;

use swiftwin_core::{LogEntry, LogLevel, LogQuery, Result};

pub struct LogBuffer {
    inner: Mutex<Inner>,
    cv: Condvar,
    debug_cap: usize,
    other_cap: usize,
}

struct Inner {
    /// (sequence number, entry), oldest first.
    debug: VecDeque<(u64, LogEntry)>,
    other: VecDeque<(u64, LogEntry)>,
    /// Total entries ever pushed (sequence number of the next entry).
    total: u64,
}

impl Inner {
    /// Entries with sequence number >= `from` that match `q`, in the order they were pushed.
    fn matching(&self, from: u64, q: &LogQuery) -> Vec<LogEntry> {
        let pick = |ring: &VecDeque<(u64, LogEntry)>| -> Vec<(u64, LogEntry)> {
            let start = ring.partition_point(|(s, _)| *s < from);
            ring.range(start..).filter(|(_, e)| q.matches(e)).cloned().collect()
        };
        let (mut a, mut b) = (pick(&self.debug).into_iter().peekable(), pick(&self.other).into_iter().peekable());
        let mut out = Vec::new();
        loop {
            let take_a = match (a.peek(), b.peek()) {
                (Some(x), Some(y)) => x.0 < y.0,
                (Some(_), None) => true,
                (None, Some(_)) => false,
                (None, None) => return out,
            };
            out.push(if take_a { a.next() } else { b.next() }.unwrap().1);
        }
    }
}

impl std::fmt::Debug for LogBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "LogBuffer({} entries)", self.len())
    }
}

impl LogBuffer {
    /// `cap` entries in each ring.
    pub fn new(cap: usize) -> Self {
        Self::with_caps(cap, cap)
    }

    pub fn with_caps(debug_cap: usize, other_cap: usize) -> Self {
        LogBuffer {
            inner: Mutex::new(Inner { debug: VecDeque::new(), other: VecDeque::new(), total: 0 }),
            cv: Condvar::new(),
            debug_cap,
            other_cap,
        }
    }

    pub fn push(&self, e: LogEntry) {
        let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let seq = g.total;
        g.total += 1;
        let (ring, cap) =
            if e.level == LogLevel::Debug { (&mut g.debug, self.debug_cap) } else { (&mut g.other, self.other_cap) };
        ring.push_back((seq, e));
        if ring.len() > cap {
            ring.pop_front();
        }
        drop(g);
        self.cv.notify_all();
    }

    pub fn len(&self) -> usize {
        let g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        g.debug.len() + g.other.len()
    }

    /// Sequence number the next entry will get: pass it as `LogQuery::since` to read only newer ones.
    pub fn next_seq(&self) -> u64 {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).total
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Deliver the backlog (and with `follow`, new entries) to `sink` until it returns false.
    pub fn read(&self, q: &LogQuery, sink: &mut dyn FnMut(LogEntry) -> bool) -> Result<()> {
        let g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let matching = g.matching(q.since.unwrap_or(0), q);
        let mut seen = g.total;
        drop(g);
        let skip = q.limit.map(|l| matching.len().saturating_sub(l)).unwrap_or(0);
        for e in matching.into_iter().skip(skip) {
            if !sink(e) {
                return Ok(());
            }
        }
        if !q.follow {
            return Ok(());
        }
        loop {
            let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            while g.total == seen {
                g = self.cv.wait_timeout(g, Duration::from_millis(500)).unwrap_or_else(|p| p.into_inner()).0;
            }
            let new = g.matching(seen, q);
            seen = g.total;
            drop(g);
            for e in new {
                if !sink(e) {
                    return Ok(());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn at(level: LogLevel, m: &str) -> LogEntry {
        LogEntry { timestamp: String::new(), process: "p".into(), pid: 1, level, message: m.into(), subsystem: None }
    }

    fn e(m: &str) -> LogEntry {
        at(LogLevel::Info, m)
    }

    fn all(b: &LogBuffer, q: LogQuery) -> Vec<String> {
        let mut got = vec![];
        b.read(&q, &mut |x| {
            got.push(x.message);
            true
        })
        .unwrap();
        got
    }

    #[test]
    fn cap_limit_and_follow() {
        let b = Arc::new(LogBuffer::new(3));
        for i in 0..5 {
            b.push(e(&i.to_string()));
        }
        assert_eq!(b.len(), 3);
        assert_eq!(all(&b, LogQuery { limit: Some(2), ..Default::default() }), vec!["3", "4"]);

        let b2 = b.clone();
        let h = std::thread::spawn(move || {
            let mut got = vec![];
            b2.read(&LogQuery { follow: true, limit: Some(0), ..Default::default() }, &mut |x| {
                got.push(x.message);
                got.len() < 2
            })
            .unwrap();
            got
        });
        std::thread::sleep(Duration::from_millis(50));
        b.push(e("a"));
        b.push(at(LogLevel::Debug, "b"));
        assert_eq!(h.join().unwrap(), vec!["a", "b"]);
    }

    /// A flood of DEBUG lines no longer pushes out an app's notice, and order is kept.
    #[test]
    fn debug_floods_keep_other_levels() {
        let b = LogBuffer::new(3);
        b.push(at(LogLevel::Debug, "d0"));
        b.push(at(LogLevel::Notice, "Button tapped"));
        for i in 1..=10 {
            b.push(at(LogLevel::Debug, &format!("d{i}")));
        }
        b.push(at(LogLevel::Error, "boom"));
        assert_eq!(b.len(), 5);
        assert_eq!(all(&b, LogQuery::default()), vec!["Button tapped", "d8", "d9", "d10", "boom"]);
        let q = LogQuery { contains: Some("tapped".into()), ..Default::default() };
        assert_eq!(all(&b, q), vec!["Button tapped"]);
    }

    #[test]
    fn since_reads_only_newer_entries() {
        let b = LogBuffer::new(10);
        b.push(e("old"));
        let cursor = b.next_seq();
        b.push(e("new"));
        assert_eq!(all(&b, LogQuery { since: Some(cursor), ..Default::default() }), vec!["new"]);
        assert!(all(&b, LogQuery { since: Some(b.next_seq()), ..Default::default() }).is_empty());
    }
}
