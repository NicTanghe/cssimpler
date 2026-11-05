//! Single-primary-finger touch input. Swipes scroll; a cancelled/swiped contact
//! cannot click, and synthetic mouse events cannot duplicate a touch action.
use std::time::{Duration, Instant};
use winit::event::TouchPhase;

#[derive(Clone, Copy, Debug)]
struct Contact {
    id: u64,
    start: (f32, f32),
    last: (f32, f32),
    swiping: bool,
}
#[derive(Default)]
pub(crate) struct TouchController {
    contact: Option<Contact>,
    suppress_until: Option<Instant>,
}
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct TouchUpdate {
    pub position: (f32, f32),
    pub press: bool,
    pub release: bool,
    pub cancel_click: bool,
    pub scroll: Option<(f32, f32)>,
}
impl TouchController {
    pub fn ignore_mouse(&self, now: Instant) -> bool {
        self.contact.is_some() || self.suppress_until.is_some_and(|until| now < until)
    }
    pub fn cancel(&mut self) {
        self.contact = None;
    }
    pub fn update(
        &mut self,
        id: u64,
        phase: TouchPhase,
        position: (f32, f32),
        now: Instant,
    ) -> Option<TouchUpdate> {
        self.suppress_until = Some(now + Duration::from_millis(700));
        if phase == TouchPhase::Started {
            if self.contact.is_some() {
                return None;
            }
            self.contact = Some(Contact {
                id,
                start: position,
                last: position,
                swiping: false,
            });
            return Some(TouchUpdate {
                position,
                press: true,
                ..Default::default()
            });
        }
        let contact = self.contact.as_mut()?;
        if contact.id != id {
            return None;
        }
        match phase {
            TouchPhase::Moved => {
                let was_swiping = contact.swiping;
                contact.swiping |= (position.0 - contact.start.0).abs()
                    + (position.1 - contact.start.1).abs()
                    > 10.0;
                let scroll = contact
                    .swiping
                    .then_some((position.0 - contact.last.0, position.1 - contact.last.1));
                let update = TouchUpdate {
                    position,
                    cancel_click: contact.swiping,
                    release: contact.swiping && !was_swiping,
                    scroll,
                    ..Default::default()
                };
                contact.last = position;
                Some(update)
            }
            TouchPhase::Ended | TouchPhase::Cancelled => {
                let cancel_click = phase == TouchPhase::Cancelled || contact.swiping;
                self.contact = None;
                Some(TouchUpdate {
                    position,
                    release: true,
                    cancel_click,
                    ..Default::default()
                })
            }
            _ => None,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tap_releases_once_and_suppresses_synthetic_mouse() {
        let now = Instant::now();
        let mut t = TouchController::default();
        assert!(
            t.update(1, TouchPhase::Started, (10.0, 20.0), now)
                .unwrap()
                .press
        );
        let end = t.update(1, TouchPhase::Ended, (10.0, 20.0), now).unwrap();
        assert!(end.release && !end.cancel_click);
        assert!(t.update(1, TouchPhase::Ended, (10.0, 20.0), now).is_none());
        assert!(t.ignore_mouse(now));
        assert!(!t.ignore_mouse(now + Duration::from_secs(1)));
    }
    #[test]
    fn swipe_cancels_click_and_ignores_other_fingers() {
        let now = Instant::now();
        let mut t = TouchController::default();
        t.update(1, TouchPhase::Started, (10.0, 20.0), now);
        assert!(t.update(2, TouchPhase::Started, (0.0, 0.0), now).is_none());
        assert!(t.update(2, TouchPhase::Ended, (0.0, 0.0), now).is_none());
        let moved = t.update(1, TouchPhase::Moved, (10.0, 60.0), now).unwrap();
        assert!(moved.cancel_click && moved.release);
        assert_eq!(moved.scroll, Some((0.0, 40.0)));
        assert!(
            t.update(1, TouchPhase::Ended, (10.0, 60.0), now)
                .unwrap()
                .cancel_click
        );
    }
    #[test]
    fn cancelled_contact_never_clicks() {
        let now = Instant::now();
        let mut t = TouchController::default();
        t.update(1, TouchPhase::Started, (10.0, 20.0), now);
        assert!(
            t.update(1, TouchPhase::Cancelled, (10.0, 20.0), now)
                .unwrap()
                .cancel_click
        );
        t.update(1, TouchPhase::Started, (10.0, 20.0), now);
        t.cancel();
        assert!(t.update(1, TouchPhase::Ended, (10.0, 20.0), now).is_none());
    }
}
