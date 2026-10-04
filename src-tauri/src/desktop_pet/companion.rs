//! Local companion chatter. Never sends a model request or changes run state.
use super::Mood;
use crate::usage::TodayUsage;
use std::time::{Duration, Instant};

const CHATTER_INTERVAL: Duration = Duration::from_secs(240);
const BUBBLE_TIME: Duration = Duration::from_secs(10);

struct Bubble {
    text: String,
    until: Instant,
    mood: Mood,
    english: bool,
}

pub(super) struct Companion {
    bubble: Option<Bubble>,
    next_chatter: Instant,
    last_poke: Option<Instant>,
    sequence: usize,
    pub(super) revision: u64,
    loading_usage: bool,
    usage_is_cost: bool,
}

impl Companion {
    pub(super) fn new(now: Instant) -> Self {
        Self {
            bubble: None,
            next_chatter: now + Duration::from_secs(8),
            last_poke: None,
            sequence: 0,
            revision: 0,
            loading_usage: false,
            usage_is_cost: false,
        }
    }

    fn clear(&mut self) {
        self.bubble = None;
        self.loading_usage = false;
        self.revision = self.revision.wrapping_add(1);
    }

    pub(super) fn dismiss(&mut self, now: Instant) {
        self.clear();
        self.next_chatter = now + CHATTER_INTERVAL;
    }

    // Busy states reset the idle interval, rather than queuing chatter for completion.
    pub(super) fn tick(
        &mut self,
        now: Instant,
        mood: Mood,
        english: bool,
        interacting: bool,
    ) -> bool {
        if interacting {
            self.dismiss(now);
            return false;
        }
        if self.bubble.as_ref().is_some_and(|bubble| {
            now >= bubble.until || bubble.mood != mood || bubble.english != english
        }) {
            self.clear();
        }
        if mood != Mood::Idle {
            self.next_chatter = now + CHATTER_INTERVAL;
            return false;
        }
        if now >= self.next_chatter && self.bubble.is_none() {
            return self.speak(now, mood, english);
        }
        false
    }

    pub(super) fn poke(&mut self, now: Instant, mood: Mood, english: bool) -> bool {
        if self
            .last_poke
            .is_some_and(|last| now.duration_since(last) < Duration::from_secs(2))
        {
            return false;
        }
        self.last_poke = Some(now);
        self.speak(now, mood, english)
    }

    fn speak(&mut self, now: Instant, mood: Mood, english: bool) -> bool {
        self.clear();
        self.next_chatter = now + CHATTER_INTERVAL;
        let text = match mood {
            Mood::Idle => {
                let choice = self.sequence % 6;
                self.sequence = self.sequence.wrapping_add(1);
                self.loading_usage = choice == 1 || choice == 4;
                self.usage_is_cost = choice == 4;
                match (choice, english) {
                    (0, false) => "我在呢，点我聊两句。",
                    (0, true) => "I'm here. Tap me to say hello.",
                    (1 | 4, false) => "翻翻今天的小账本……",
                    (1 | 4, true) => "Checking today's little ledger…",
                    (2, false) => "眯一小会儿，有事叫我。",
                    (2, true) => "Resting my eyes. Give me a nudge.",
                    (3, false) => "伸个懒腰吧，我陪你。",
                    (3, true) => "Time for a little stretch?",
                    (_, false) => "陪你发会儿呆也挺好。",
                    (_, true) => "A quiet moment together is nice.",
                }
            }
            Mood::Waiting if english => "Need your input. Double-click me.",
            Mood::Waiting => "等你确认呢，双击我看看。",
            Mood::Error if english => "Something went wrong. Double-click me.",
            Mood::Error => "遇到问题了，双击我看看。",
            Mood::Done if english => "All done. Come take a look!",
            Mood::Done => "做完啦，来看看吧。",
            _ if english => "Still on it. Double-click to check in.",
            _ => "还在忙呢，双击我看进展。",
        };
        self.bubble = Some(Bubble {
            text: text.into(),
            until: now + BUBBLE_TIME,
            mood,
            english,
        });
        self.loading_usage
    }

    pub(super) fn text(&self) -> Option<&str> {
        self.bubble.as_ref().map(|bubble| bubble.text.as_str())
    }

    pub(super) fn finish_usage(
        &mut self,
        revision: u64,
        result: Result<TodayUsage, String>,
        now: Instant,
    ) {
        if self.revision != revision || !self.loading_usage {
            return;
        }
        self.loading_usage = false;
        let Some(bubble) = self.bubble.as_mut().filter(|bubble| now < bubble.until) else {
            return;
        };
        bubble.text = match result {
            Ok(usage) if usage.date == chrono::Local::now().date_naive() => {
                usage_text(&usage, bubble.english, self.usage_is_cost)
            }
            _ if bubble.english => "Couldn't read today's usage just now.".into(),
            _ => "今天的用量暂时没读到。".into(),
        };
        bubble.until = now + BUBBLE_TIME;
    }
}

fn usage_text(usage: &TodayUsage, english: bool, cost: bool) -> String {
    let s = &usage.summary;
    if s.total_requests == 0 {
        return match (usage.skipped_records > 0, english) {
            (true, true) => "Today's records are incomplete.",
            (true, false) => "今天的记录没读全，先不报数啦。",
            (false, true) => "No calls recorded today yet.",
            (false, false) => "今天还没有记录到调用呢。",
        }
        .into();
    }
    if cost {
        if usage.missing_cost_requests == s.total_requests as usize {
            return if english {
                "Cost unknown — not necessarily free."
            } else {
                "费用还没拿到，不代表免费哦。"
            }
            .into();
        }
        let incomplete = usage.missing_cost_requests > 0 || usage.skipped_records > 0;
        return match (english, incomplete) {
            (true, true) => format!("Recorded today: ~${:.4} USD; incomplete.", s.total_cost_usd),
            (true, false) => format!("Recorded today: ~${:.4} USD.", s.total_cost_usd),
            (false, true) => format!("今天已记录约 ${:.4}，部分费用缺失。", s.total_cost_usd),
            (false, false) => format!("今天已记录费用约 ${:.4} 美元。", s.total_cost_usd),
        };
    }
    if english {
        format!(
            "{} tokens recorded today{}.",
            s.total_tokens,
            if s.missing_usage_requests > 0 || usage.skipped_records > 0 {
                " (incomplete)"
            } else {
                ""
            }
        )
    } else {
        format!(
            "今天已记录 {} Token{}。",
            s.total_tokens,
            if s.missing_usage_requests > 0 || usage.skipped_records > 0 {
                "（数据不全）"
            } else {
                ""
            }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::UsageSummary;

    #[test]
    fn busy_work_suppresses_chatter_and_stale_usage() {
        let now = Instant::now();
        let mut companion = Companion::new(now);
        assert!(!companion.poke(now, Mood::Idle, false));
        assert!(companion.poke(now + Duration::from_secs(3), Mood::Idle, false));
        let revision = companion.revision;
        assert!(!companion.tick(now + Duration::from_secs(4), Mood::Waiting, false, false));
        companion.finish_usage(revision, Err("late".into()), now + Duration::from_secs(5));
        assert!(companion.text().is_none());
        assert!(!companion.tick(now + Duration::from_secs(30), Mood::Idle, false, false));
        assert!(!companion.poke(now + Duration::from_secs(31), Mood::Waiting, false));
    }

    #[test]
    fn bubbles_expire_and_dismiss_invalidates_pending_usage() {
        let now = Instant::now();
        let mut companion = Companion::new(now);
        companion.poke(now, Mood::Idle, true);
        companion.tick(now + Duration::from_secs(11), Mood::Idle, true, false);
        assert!(companion.text().is_none());
        assert!(companion.poke(now + Duration::from_secs(12), Mood::Idle, true));
        let revision = companion.revision;
        companion.dismiss(now + Duration::from_secs(13));
        companion.finish_usage(revision, Err("late".into()), now + Duration::from_secs(14));
        assert!(companion.text().is_none());
    }

    #[test]
    fn missing_costs_never_claim_free_usage() {
        let mut usage = TodayUsage {
            date: chrono::Local::now().date_naive(),
            summary: UsageSummary {
                total_requests: 2,
                total_tokens: 4321,
                ..Default::default()
            },
            missing_cost_requests: 2,
            skipped_records: 0,
        };
        assert!(!usage_text(&usage, false, true).contains("$0.0000"));
        assert!(!usage_text(&usage, true, true).contains("$0.0000"));
        usage.missing_cost_requests = 1;
        usage.summary.total_cost_usd = 0.1234;
        assert!(usage_text(&usage, true, true).contains("$0.1234"));
        assert!(usage_text(&usage, true, false).contains("4321"));
    }
}
