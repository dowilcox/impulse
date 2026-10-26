use gpui::{Context, Pixels, Task, px};
use instant::Duration;

use crate::input::CursorBlinking;

static INTERVAL: Duration = Duration::from_millis(500);
static PAUSE_DELAY: Duration = Duration::from_millis(300);

pub(super) const CURSOR_WIDTH: Pixels = px(1.);

/// To manage the Input cursor blinking.
///
/// It will start blinking with a interval of 500ms.
/// Every loop will notify the view to update the `visible`, and Input will observe this update to touch repaint.
///
/// The input painter will check if this in visible state, then it will draw the cursor.
pub(crate) struct BlinkCursor {
    visible: bool,
    paused: bool,
    epoch: usize,
    /// Impulse: how the caret blinks (`set_cursor_blinking`).
    blinking: CursorBlinking,
    /// Impulse: how much of a fading or expanding caret shows, 0 to 1.
    presence: f32,

    _task: Task<()>,
}

impl BlinkCursor {
    pub(crate) fn new() -> Self {
        Self {
            visible: false,
            paused: false,
            epoch: 0,
            blinking: CursorBlinking::default(),
            presence: 0.,
            _task: Task::ready(()),
        }
    }

    /// Impulse: change how the caret blinks, restarting a running cycle.
    pub(crate) fn set_blinking(&mut self, blinking: CursorBlinking, cx: &mut Context<Self>) {
        if self.blinking == blinking {
            return;
        }
        self.blinking = blinking;
        if self.epoch != 0 {
            self.start(cx);
        }
    }

    pub(crate) fn blinking(&self) -> CursorBlinking {
        self.blinking
    }

    /// Impulse: how much of the caret shows, 0 to 1: all or nothing unless
    /// it fades or expands.
    pub(crate) fn presence(&self) -> f32 {
        if self.paused {
            1.
        } else if matches!(self.blinking, CursorBlinking::Blink | CursorBlinking::Solid) {
            if self.visible { 1. } else { 0. }
        } else {
            self.presence
        }
    }

    /// Impulse: (re)start the cycle `blinking` asks for, from a shown caret.
    fn resume(&mut self, epoch: usize, cx: &mut Context<Self>) {
        match self.blinking {
            CursorBlinking::Blink => self.blink(epoch, cx),
            CursorBlinking::Solid => {
                self.visible = true;
                cx.notify();
            }
            _ => self.animate(epoch, Duration::ZERO, cx),
        }
    }

    /// Impulse: one step of a fading or expanding caret, `elapsed` into it.
    fn animate(&mut self, epoch: usize, elapsed: Duration, cx: &mut Context<Self>) {
        if epoch != self.epoch {
            return;
        }
        if self.paused {
            self.visible = true;
            return;
        }
        let (presence, next) = animated_presence(self.blinking, elapsed);
        if presence != self.presence || !self.visible {
            self.presence = presence;
            self.visible = presence > 0.;
            cx.notify();
        }
        // Visible for good once the run is over, as Monaco's is.
        let Some(next) = next else {
            self.visible = true;
            return;
        };
        let epoch = self.next_epoch();
        self._task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(next).await;
            if let Some(this) = this.upgrade() {
                this.update(cx, |this, cx| this.animate(epoch, elapsed + next, cx));
            }
        });
    }

    /// Start the blinking from a visible cursor.
    ///
    /// A single focus can start the cursor more than once: `InputState::focus`
    /// starts it and then the focus observer starts it again, and window
    /// activation starts it too. Resuming the running cycle would toggle the
    /// cursor that was just shown off again, so every start restarts the cycle.
    pub(crate) fn start(&mut self, cx: &mut Context<Self>) {
        self.paused = false;
        self.visible = false;
        let epoch = self.next_epoch();
        // Impulse: the cycle the host chose; `Blink` is GPUI Kit's.
        self.resume(epoch, cx);
    }

    /// Stop the blinking and clear the blink state, so the next [`Self::start`]
    /// begins from a visible cursor instead of resuming a stale pause.
    pub(crate) fn stop(&mut self, cx: &mut Context<Self>) {
        self.epoch = 0;
        self.paused = false;
        self.visible = false;
        self.presence = 0.;
        self._task = Task::ready(());
        cx.notify();
    }

    fn next_epoch(&mut self) -> usize {
        self.epoch += 1;
        self.epoch
    }

    fn blink(&mut self, epoch: usize, cx: &mut Context<Self>) {
        // A task from an earlier blink lifecycle must not mutate the current state.
        if epoch != self.epoch {
            return;
        }

        if self.paused {
            self.visible = true;
            return;
        }

        self.visible = !self.visible;
        cx.notify();

        // Schedule the next blink
        let epoch = self.next_epoch();
        self._task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(INTERVAL).await;
            if let Some(this) = this.upgrade() {
                this.update(cx, |this, cx| this.blink(epoch, cx));
            }
        });
    }

    pub(crate) fn visible(&self) -> bool {
        // Keep showing the cursor if paused
        self.paused || self.visible
    }

    /// Show the cursor immediately and restart the idle delay before blinking resumes.
    ///
    /// This is a no-op on a cursor that is not blinking. `epoch` is zero only
    /// before the first [`Self::start`] and after [`Self::stop`], so a zero
    /// epoch means the input is not focused, and there is no cursor on screen
    /// to keep visible. Pausing it anyway would start a blink loop that no blur
    /// is left to stop, and every blink repaints the view that the input is in.
    pub(crate) fn pause(&mut self, cx: &mut Context<Self>) {
        if self.epoch == 0 {
            return;
        }

        self.paused = true;
        self.visible = true;
        cx.notify();

        // Every pause replaces the pending timer, keeping repeated input visible.
        let epoch = self.next_epoch();
        self._task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(PAUSE_DELAY).await;

            if let Some(this) = this.upgrade() {
                this.update(cx, |this, cx| {
                    if epoch != this.epoch {
                        return;
                    }

                    this.paused = false;
                    // Impulse: restart the host's cycle, not always a blink.
                    this.resume(epoch, cx);
                });
            }
        });
    }
}

/// Impulse: Monaco's fading and expanding blinks, as its CSS animates them:
/// half a second each way, alternating, 20 times, then shown for good. Each
/// way holds shown for its first 100 ms, then eases out until `end`.
const ANIMATION_HALF: Duration = Duration::from_millis(500);
const ANIMATION_HALVES: u32 = 20;
const ANIMATION_HOLD: Duration = Duration::from_millis(100);
/// How often a fading caret repaints: the whole editor repaints with it.
const ANIMATION_FRAME: Duration = Duration::from_millis(33);

/// How much of the caret shows `elapsed` into a fading or expanding blink,
/// and how long until it next changes (`None`: never again).
fn animated_presence(blinking: CursorBlinking, elapsed: Duration) -> (f32, Option<Duration>) {
    let end = match blinking {
        CursorBlinking::Smooth => Duration::from_millis(300),
        CursorBlinking::Phase => Duration::from_millis(450),
        CursorBlinking::Expand => Duration::from_millis(400),
        CursorBlinking::Blink | CursorBlinking::Solid => return (1., None),
    };
    let half = ANIMATION_HALF.as_millis() as u64;
    let elapsed = elapsed.as_millis() as u64;
    if elapsed >= half * ANIMATION_HALVES as u64 {
        return (1., None);
    }
    let within = elapsed % half;
    let forward = (elapsed / half) % 2 == 0;
    let (hold, end) = (ANIMATION_HOLD.as_millis() as u64, end.as_millis() as u64);
    // Where this half's keyframe time is, and the stretch of it that moves.
    let (time, moving) = if forward {
        (within, hold..end)
    } else {
        (half - within, (half - end)..(half - hold))
    };
    let presence = if time <= hold {
        1.
    } else if time >= end {
        0.
    } else {
        let t = (time - hold) as f32 / (end - hold) as f32;
        // CSS's ease-in-out, near enough.
        1. - t * t * (3. - 2. * t)
    };
    let next = if within < moving.start {
        moving.start - within
    } else if within < moving.end {
        ANIMATION_FRAME.as_millis() as u64
    } else {
        half - within
    };
    (presence, Some(Duration::from_millis(next.max(1))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{AppContext as _, TestAppContext};
    use std::{cell::Cell, rc::Rc};

    #[gpui::test]
    fn repeated_pauses_keep_cursor_visible_until_idle(cx: &mut TestAppContext) {
        let cursor = cx.new(|_| BlinkCursor::new());
        assert!(!cursor.read_with(cx, |cursor, _| cursor.visible()));
        // Only a focused input blinks, and only a blinking cursor pauses.
        cursor.update(cx, |cursor, cx| cursor.start(cx));
        cx.run_until_parked();
        for _ in 0..5 {
            cursor.update(cx, |cursor, cx| cursor.pause(cx));
            cx.run_until_parked();
            cx.executor().advance_clock(Duration::from_millis(200));
            cx.run_until_parked();
            assert!(cursor.read_with(cx, |cursor, _| cursor.visible()));
        }
        cx.executor().advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
        assert!(!cursor.read_with(cx, |cursor, _| cursor.visible()));
        cx.executor().advance_clock(INTERVAL);
        cx.run_until_parked();
        assert!(cursor.read_with(cx, |cursor, _| cursor.visible()));
    }

    #[gpui::test]
    fn starting_a_blinking_cursor_keeps_it_visible(cx: &mut TestAppContext) {
        let cursor = cx.new(|_| BlinkCursor::new());

        // Clicking into an input starts the cursor from `focus` and again from
        // the focus observer; the second start must not hide the new cursor.
        cursor.update(cx, |cursor, cx| cursor.start(cx));
        cursor.update(cx, |cursor, cx| cursor.start(cx));
        cx.run_until_parked();
        assert!(cursor.read_with(cx, |cursor, _| cursor.visible()));

        // The restart drops the first cycle's timer, so the cursor stays
        // visible for a full interval before it blinks off.
        cx.executor()
            .advance_clock(INTERVAL - Duration::from_millis(1));
        cx.run_until_parked();
        assert!(cursor.read_with(cx, |cursor, _| cursor.visible()));
        cx.executor().advance_clock(Duration::from_millis(1));
        cx.run_until_parked();
        assert!(!cursor.read_with(cx, |cursor, _| cursor.visible()));
    }

    #[gpui::test]
    fn pausing_a_cursor_that_is_not_blinking_does_not_start_it(cx: &mut TestAppContext) {
        // Never focused, so `start` was never called: the epoch is still zero.
        let cursor = cx.new(|_| BlinkCursor::new());

        // What a programmatic `set_value` on an unfocused input does.
        cursor.update(cx, |cursor, cx| cursor.pause(cx));
        cx.run_until_parked();
        cx.executor().advance_clock(PAUSE_DELAY);
        cx.run_until_parked();
        assert!(!cursor.read_with(cx, |cursor, _| cursor.visible()));

        let notifies = Rc::new(Cell::new(0usize));
        let counter = notifies.clone();
        let _observer =
            cx.update(|cx| cx.observe(&cursor, move |_, _| counter.set(counter.get() + 1)));
        cx.run_until_parked();

        cx.executor().advance_clock(INTERVAL * 6);
        cx.run_until_parked();

        assert_eq!(
            notifies.get(),
            0,
            "a cursor that was never started is blinking, and every blink repaints the view"
        );
    }

    #[gpui::test]
    fn blurring_a_paused_cursor_leaves_the_next_focus_blinking(cx: &mut TestAppContext) {
        let cursor = cx.new(|_| BlinkCursor::new());
        cursor.update(cx, |cursor, cx| cursor.start(cx));
        cx.run_until_parked();

        // Typing pauses the blink, then the input is blurred before the pause
        // elapses: tabbing away right after a keystroke does exactly this.
        cursor.update(cx, |cursor, cx| cursor.pause(cx));
        cx.run_until_parked();
        cursor.update(cx, |cursor, cx| cursor.stop(cx));
        cx.run_until_parked();
        assert!(!cursor.read_with(cx, |cursor, _| cursor.visible()));

        // Focusing again shows the cursor and blinks it, rather than leaving a
        // stale pause to swallow the start.
        cursor.update(cx, |cursor, cx| cursor.start(cx));
        cx.run_until_parked();
        assert!(cursor.read_with(cx, |cursor, _| cursor.visible()));
        cx.executor().advance_clock(INTERVAL);
        cx.run_until_parked();
        assert!(!cursor.read_with(cx, |cursor, _| cursor.visible()));
    }

    #[gpui::test]
    fn stopping_a_paused_cursor_ends_the_blink_loop(cx: &mut TestAppContext) {
        let cursor = cx.new(|_| BlinkCursor::new());
        cursor.update(cx, |cursor, cx| cursor.start(cx));
        cx.run_until_parked();
        cursor.update(cx, |cursor, cx| cursor.pause(cx));
        cx.run_until_parked();
        cursor.update(cx, |cursor, cx| cursor.stop(cx));
        cx.run_until_parked();

        let notifications = Rc::new(Cell::new(0usize));
        let count = notifications.clone();
        let _observer = cx.update(|cx| cx.observe(&cursor, move |_, _| count.set(count.get() + 1)));
        cx.run_until_parked();

        cx.executor().advance_clock(Duration::from_secs(3));
        cx.run_until_parked();

        assert!(!cursor.read_with(cx, |cursor, _| cursor.visible()));
        assert_eq!(notifications.get(), 0, "a stopped cursor kept blinking");

        cursor.update(cx, |cursor, cx| cursor.start(cx));
        cx.run_until_parked();
        assert!(cursor.read_with(cx, |cursor, _| cursor.visible()));
    }

    #[gpui::test]
    fn stopping_a_blinking_cursor_ends_the_blink_loop(cx: &mut TestAppContext) {
        let cursor = cx.new(|_| BlinkCursor::new());
        cursor.update(cx, |cursor, cx| cursor.start(cx));
        cx.run_until_parked();
        cursor.update(cx, |cursor, cx| cursor.stop(cx));
        cx.run_until_parked();

        let notifications = Rc::new(Cell::new(0usize));
        let count = notifications.clone();
        let _observer = cx.update(|cx| cx.observe(&cursor, move |_, _| count.set(count.get() + 1)));
        cx.run_until_parked();

        cx.executor().advance_clock(Duration::from_secs(3));
        cx.run_until_parked();

        assert!(!cursor.read_with(cx, |cursor, _| cursor.visible()));
        assert_eq!(notifications.get(), 0, "a stopped cursor kept blinking");
    }

    #[test]
    fn a_smooth_caret_fades_out_and_back_in_then_stays() {
        let at = |ms| animated_presence(CursorBlinking::Smooth, Duration::from_millis(ms));
        assert_eq!(at(0), (1., Some(Duration::from_millis(100))));
        let (fading, next) = at(200);
        assert!(fading > 0. && fading < 1., "{fading}");
        assert_eq!(next, Some(ANIMATION_FRAME));
        assert_eq!(at(400), (0., Some(Duration::from_millis(100))));
        // On the way back, hidden until it fades in.
        assert_eq!(at(600).0, 0.);
        assert!(at(800).0 > 0.);
        assert_eq!(at(950).0, 1.);
        assert_eq!(at(10_000), (1., None));
    }

    #[gpui::test]
    fn a_solid_caret_never_blinks(cx: &mut TestAppContext) {
        let cursor = cx.new(|_| BlinkCursor::new());
        cursor.update(cx, |cursor, cx| {
            cursor.set_blinking(CursorBlinking::Solid, cx);
            cursor.start(cx);
        });
        cx.run_until_parked();
        for _ in 0..4 {
            cx.executor().advance_clock(INTERVAL);
            cx.run_until_parked();
            assert!(cursor.read_with(cx, |cursor, _| cursor.visible()));
        }
    }

    #[gpui::test]
    fn an_expanding_caret_shrinks_then_grows(cx: &mut TestAppContext) {
        let cursor = cx.new(|_| BlinkCursor::new());
        cursor.update(cx, |cursor, cx| {
            cursor.set_blinking(CursorBlinking::Expand, cx);
            cursor.start(cx);
        });
        cx.run_until_parked();
        assert_eq!(cursor.read_with(cx, |cursor, _| cursor.presence()), 1.);
        cx.executor().advance_clock(Duration::from_millis(450));
        cx.run_until_parked();
        assert_eq!(cursor.read_with(cx, |cursor, _| cursor.presence()), 0.);
        cx.executor().advance_clock(Duration::from_millis(500));
        cx.run_until_parked();
        assert_eq!(cursor.read_with(cx, |cursor, _| cursor.presence()), 1.);
    }
}
