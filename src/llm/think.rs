//! Splitting the model's reasoning out of the reply it streams.

use std::sync::mpsc::Sender;

use crate::state::GenMsg;

/// Take the model's reasoning out of a reply.
///
/// A paired `<think>\u{2026}</think>` block goes as a unit. A bare `</think>` counts
/// too, and discards everything before it: some chat templates open the block
/// themselves, so the reply arrives as reasoning, a closing tag, and only then
/// the answer.
pub fn strip_think_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    loop {
        let open = rest.find(OPEN);
        let close = rest.find(CLOSE);
        // A close with no open before it was opened before the reply started,
        // so everything up to it is reasoning — including whatever has been
        // taken for output so far.
        if let Some(j) = close
            && open.is_none_or(|i| j < i)
        {
            out.clear();
            rest = &rest[j + CLOSE.len()..];
            continue;
        }
        let Some(i) = open else { break };
        out.push_str(&rest[..i]);
        rest = &rest[i + OPEN.len()..];
        match rest.find(CLOSE) {
            Some(j) => rest = &rest[j + CLOSE.len()..],
            // An unterminated block runs to the end of the reply.
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

const OPEN: &str = "<think>";
const CLOSE: &str = "</think>";

#[derive(Default)]
pub struct ThinkSplit {
    in_think: bool,
    hold: String,
}

impl ThinkSplit {
    /// Feed a chunk; returns `Ok((think_bytes_added, out_bytes_added))`.
    pub fn feed(
        &mut self,
        chunk: &str,
        tx: &Sender<GenMsg>,
        full_output: &mut String,
    ) -> std::result::Result<(usize, usize), ()> {
        self.hold.push_str(chunk);
        let mut think_added = 0usize;
        let mut out_added = 0usize;
        loop {
            if self.in_think {
                if let Some(pos) = self.hold.find(CLOSE) {
                    let part: String = self.hold.drain(..pos).collect();
                    self.hold.drain(..CLOSE.len());
                    if !part.is_empty() {
                        think_added += part.len();
                        if tx.send(GenMsg::Thinking(part)).is_err() {
                            return Err(());
                        }
                    }
                    self.in_think = false;
                } else {
                    let keep = partial_tail_len(&self.hold, CLOSE);
                    let flush_len = self.hold.len() - keep;
                    if flush_len > 0 {
                        let flush: String = self.hold.drain(..flush_len).collect();
                        think_added += flush.len();
                        if tx.send(GenMsg::Thinking(flush)).is_err() {
                            return Err(());
                        }
                    }
                    return Ok((think_added, out_added));
                }
            } else if let Some(pos) = self.hold.find(CLOSE)
                && self.hold.find(OPEN).is_none_or(|open| pos < open)
            {
                // A close with no open before it: the model was already
                // reasoning when the reply began, so everything sent as output
                // so far was its draft, not the answer.
                let part: String = self.hold.drain(..pos).collect();
                self.hold.drain(..CLOSE.len());
                think_added += part.len();
                full_output.clear();
                if tx.send(GenMsg::Reset).is_err() {
                    return Err(());
                }
                if !part.is_empty() && tx.send(GenMsg::Thinking(part)).is_err() {
                    return Err(());
                }
            } else if let Some(pos) = self.hold.find(OPEN) {
                let part: String = self.hold.drain(..pos).collect();
                self.hold.drain(..OPEN.len());
                if !part.is_empty() {
                    out_added += part.len();
                    full_output.push_str(&part);
                    if tx.send(GenMsg::Output(part)).is_err() {
                        return Err(());
                    }
                }
                self.in_think = true;
            } else {
                // Either tag may be split across chunks, so hold back enough
                // for the longer of the two prefixes.
                let keep =
                    partial_tail_len(&self.hold, OPEN).max(partial_tail_len(&self.hold, CLOSE));
                let flush_len = self.hold.len() - keep;
                if flush_len > 0 {
                    let flush: String = self.hold.drain(..flush_len).collect();
                    out_added += flush.len();
                    full_output.push_str(&flush);
                    if tx.send(GenMsg::Output(flush)).is_err() {
                        return Err(());
                    }
                }
                return Ok((think_added, out_added));
            }
        }
    }

    /// Flush remaining held bytes; returns `Ok((think_bytes_added, out_bytes_added))`.
    pub fn flush(
        &mut self,
        tx: &Sender<GenMsg>,
        full_output: &mut String,
    ) -> std::result::Result<(usize, usize), ()> {
        if self.hold.is_empty() {
            return Ok((0, 0));
        }
        let tail: String = self.hold.drain(..).collect();
        let len = tail.len();
        if self.in_think {
            if tx.send(GenMsg::Thinking(tail)).is_err() {
                return Err(());
            }
            Ok((len, 0))
        } else {
            full_output.push_str(&tail);
            if tx.send(GenMsg::Output(tail)).is_err() {
                return Err(());
            }
            Ok((0, len))
        }
    }
}

fn partial_tail_len(s: &str, tag: &str) -> usize {
    let max = tag.len().saturating_sub(1).min(s.len());
    let sb = s.as_bytes();
    let tb = tag.as_bytes();
    for n in (1..=max).rev() {
        if sb.ends_with(&tb[..n]) {
            return n;
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;

    #[test]
    fn strip_think_tags_removes_paired_blocks() {
        assert_eq!(
            strip_think_tags("before<think>planning</think>after"),
            "beforeafter"
        );
    }

    #[test]
    fn strip_think_tags_drops_unterminated_tail() {
        assert_eq!(strip_think_tags("keep<think>unterminated"), "keep");
    }

    /// The shape qwen actually produces: the template opened the block, so only
    /// the closing tag reaches us, with the model's draft answer ahead of it.
    #[test]
    fn strip_think_tags_drops_a_draft_ending_in_a_bare_close() {
        const REPLY: &str = "\
feat(session): add sessions

Draft body.
</think>

feat(session): add sessions

Real body.
";

        assert_eq!(
            strip_think_tags(REPLY),
            "\n\nfeat(session): add sessions\n\nReal body.\n"
        );
    }

    #[test]
    fn a_bare_close_wins_over_a_later_paired_block() {
        assert_eq!(
            strip_think_tags("draft</think>answer<think>more planning</think> tail"),
            "answer tail"
        );
    }

    #[test]
    fn strip_think_tags_leaves_a_reply_with_no_tags_alone() {
        assert_eq!(
            strip_think_tags("fix(git): stage untracked files"),
            "fix(git): stage untracked files"
        );
    }

    #[test]
    fn think_split_reclassifies_a_draft_when_a_bare_close_arrives() {
        let (tx, rx) = channel::<GenMsg>();
        let mut p = ThinkSplit::default();
        let mut out = String::new();
        p.feed("fix: draft subject\n", &tx, &mut out).unwrap();
        assert_eq!(out, "fix: draft subject\n", "the draft streams as output");
        p.feed("</think>fix: real subject", &tx, &mut out).unwrap();
        p.flush(&tx, &mut out).unwrap();
        drop(tx);

        assert_eq!(out, "fix: real subject", "the draft was taken back");
        let msgs: Vec<GenMsg> = rx.iter().collect();
        assert!(
            msgs.iter().any(|msg| matches!(msg, GenMsg::Reset)),
            "consumers are told to drop what they have: {msgs:?}"
        );
        assert!(matches!(&msgs[0], GenMsg::Output(s) if s == "fix: draft subject\n"));
        assert!(matches!(msgs.last(), Some(GenMsg::Output(s)) if s == "fix: real subject"));
    }

    #[test]
    fn think_split_holds_a_partial_close_across_chunks() {
        let (tx, rx) = channel::<GenMsg>();
        let mut p = ThinkSplit::default();
        let mut out = String::new();
        p.feed("draft</thi", &tx, &mut out).unwrap();
        p.feed("nk>real", &tx, &mut out).unwrap();
        p.flush(&tx, &mut out).unwrap();
        drop(tx);

        assert_eq!(out, "real", "no half tag leaked into the output");
        let msgs: Vec<GenMsg> = rx.iter().collect();
        assert!(msgs.iter().any(|msg| matches!(msg, GenMsg::Reset)));
    }

    #[test]
    fn partial_tail_detects_open_prefix() {
        assert_eq!(partial_tail_len("foo<thi", OPEN), 4);
        assert_eq!(partial_tail_len("done", OPEN), 0);
        assert_eq!(partial_tail_len("x<", OPEN), 1);
    }

    #[test]
    fn think_split_routes_inline_tags() {
        let (tx, rx) = channel::<GenMsg>();
        let mut p = ThinkSplit::default();
        let mut out = String::new();
        let (tb, ob) = p
            .feed("feat: foo<think>let me see</think> bar", &tx, &mut out)
            .unwrap();
        let (ftb, fob) = p.flush(&tx, &mut out).unwrap();
        drop(tx);
        let msgs: Vec<GenMsg> = rx.iter().collect();
        assert_eq!(out, "feat: foo bar");
        assert_eq!(tb + ftb, "let me see".len());
        assert_eq!(ob + fob, "feat: foo bar".len());
        assert!(matches!(&msgs[0], GenMsg::Output(s) if s == "feat: foo"));
        assert!(matches!(&msgs[1], GenMsg::Thinking(s) if s == "let me see"));
        assert!(matches!(&msgs[2], GenMsg::Output(s) if s == " bar"));
    }

    #[test]
    fn think_split_holds_partial_tag_across_chunks() {
        let (tx, rx) = channel::<GenMsg>();
        let mut p = ThinkSplit::default();
        let mut out = String::new();
        let (tb1, ob1) = p.feed("feat: foo<thi", &tx, &mut out).unwrap();
        let (tb2, ob2) = p.feed("nk>reason</think>ok", &tx, &mut out).unwrap();
        let (ftb, fob) = p.flush(&tx, &mut out).unwrap();
        drop(tx);
        let msgs: Vec<GenMsg> = rx.iter().collect();
        assert_eq!(out, "feat: foook");
        assert_eq!(tb1 + tb2 + ftb, "reason".len());
        assert_eq!(ob1 + ob2 + fob, "feat: foook".len());
        assert!(matches!(&msgs[0], GenMsg::Output(s) if s == "feat: foo"));
        assert!(matches!(&msgs[1], GenMsg::Thinking(s) if s == "reason"));
        assert!(matches!(&msgs[2], GenMsg::Output(s) if s == "ok"));
    }
}
