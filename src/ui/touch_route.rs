//! Multi-touch on Android: which finger egui's pointer follows.
//!
//! egui's integration (egui-winit) repeats the *first* finger down as the pointer and gives every later one touch
//! events only. So a finger held on the piano keyboard took the pointer, and a second finger could press nothing else:
//! not the rack, a mixer fader, a button. The widgets that play each finger themselves ([`crate::ui::fingers`]: the
//! keyboard, the chord keyboard, the drum pads, the piano roll's keys) register with [`takes_fingers`]; a touch that
//! goes down on one of them stays a touch, and the pointer follows the first touch that went down anywhere else
//! instead ([`TouchRoute::route`], before egui reads the frame's input). Nothing changes while no finger is on such a
//! widget: the pointer still follows the first finger down.
//!
//! The touches of those widgets' fingers move to a device of their own ([`CLAIMED`]): [`crate::ui::fingers`] reads
//! every device alike, but egui's multi-touch (a pinch: [`egui::InputState::multi_touch`]) counts the fingers of one
//! device only, so a finger on the keys and one on an editor or a button is no pinch.
//!
//! A mouse (Android takes one) is untouched in frames without touches. While a finger is down, the integration's own
//! pointer events are dropped: the route's replace them.

use std::collections::HashSet;
use std::sync::{LazyLock, Mutex, PoisonError};

use eframe::egui::{self, Event, Id, Modifiers, PointerButton, Pos2, TouchDeviceId, TouchPhase};

/// The device the touches of the fingers that [`takes_fingers`] widgets play are moved to.
pub const CLAIMED: TouchDeviceId = TouchDeviceId(0x486f_6e65_7944_6577);

/// The widgets registered with [`takes_fingers`].
fn registry() -> Id {
    Id::new("takes fingers")
}

/// The widget `id` plays each finger itself ([`crate::ui::fingers`]): a touch that goes down on it (as the top
/// interactive widget) leaves the pointer to the next finger ([`TouchRoute`]). Call it each frame it is drawn.
pub fn takes_fingers(ctx: &egui::Context, id: Id) {
    ctx.data_mut(|d| {
        d.get_temp_mut_or_default::<HashSet<Id>>(registry())
            .insert(id);
    });
}

/// Whether a touch going down at `pos` lands on a widget registered with [`takes_fingers`]: the top interactive widget
/// there, by the rects of the pass just drawn (egui's `prev_pass` between passes, as [`crate::ui::fingers`] reads it
/// during one). Call it before a pass begins ([`eframe::App::raw_input_hook`]).
pub fn on_finger_widget(ctx: &egui::Context, pos: Pos2) -> bool {
    let Some(ids) = ctx.data(|d| d.get_temp::<HashSet<Id>>(registry())) else {
        return false;
    };
    let layers: Vec<egui::LayerId> = ctx.memory(|m| m.layer_ids().collect());
    ctx.viewport(|v| {
        (layers.iter())
            .flat_map(|&layer| v.prev_pass.widgets.get_layer(layer))
            .filter(|w| w.sense.interactive() && w.interact_rect.contains(pos))
            .last()
            .is_some_and(|w| ids.contains(&w.id))
    })
}

/// The app's [`TouchRoute`] over a frame's `raw` input, as eframe hands it to [`eframe::App::raw_input_hook`]: native
/// touch screens (Android, iOS), whose integration repeats the first finger as the pointer.
pub fn route_input(ctx: &egui::Context, raw: &mut egui::RawInput) {
    static ROUTE: LazyLock<Mutex<TouchRoute>> = LazyLock::new(Mutex::default);
    let mut route = ROUTE.lock().unwrap_or_else(PoisonError::into_inner);
    route.route(&mut raw.events, raw.modifiers, |pos| on_finger_widget(ctx, pos));
}

/// The fingers down, and the one the pointer follows.
#[derive(Default, Debug)]
pub struct TouchRoute {
    /// The touches down now, by id.
    down: HashSet<u64>,
    /// The touch the pointer follows.
    pointer: Option<u64>,
    /// The touches down on a [`takes_fingers`] widget, on [`CLAIMED`].
    claimed: HashSet<u64>,
}

impl TouchRoute {
    /// Rewrites a frame's `events`: every touch event stays; the pointer follows the first touch down that
    /// `takes_fingers` (where it went down) does not claim, pressing and releasing the primary button with it; the
    /// integration's own pointer events go while any finger is down or touches this frame. A frame without touches, with
    /// none down, is left alone (a mouse). A lost window focus forgets the fingers.
    pub fn route(
        &mut self,
        events: &mut Vec<Event>,
        modifiers: Modifiers,
        takes_fingers: impl Fn(Pos2) -> bool,
    ) {
        if events
            .iter()
            .any(|e| matches!(e, Event::WindowFocused(false)))
        {
            self.forget(events);
        }
        let touches = events.iter().any(|e| matches!(e, Event::Touch { .. }));
        if !touches && self.down.is_empty() {
            return;
        }
        let button = |pos, pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers,
        };
        let mut out = Vec::with_capacity(events.len() + 3);
        for event in events.drain(..) {
            let Event::Touch {
                device_id,
                id,
                phase,
                pos,
                force,
            } = event
            else {
                // The integration's pointer, which follows the first finger: the route's replaces it.
                if !matches!(
                    event,
                    Event::PointerMoved(_) | Event::PointerButton { .. } | Event::PointerGone
                ) {
                    out.push(event);
                }
                continue;
            };
            let touch = |device_id| Event::Touch {
                device_id,
                id,
                phase,
                pos,
                force,
            };
            let n = id.0;
            if phase == TouchPhase::Start {
                // A touch id down again lost its lift (Android reuses ids): it starts afresh.
                if !self.down.insert(n) && self.pointer == Some(n) {
                    self.pointer = None;
                    out.extend([button(pos, false), Event::PointerGone]);
                }
                self.claimed.remove(&n);
                if takes_fingers(pos) {
                    self.claimed.insert(n);
                }
            }
            let claimed = self.claimed.contains(&n);
            out.push(touch(if claimed { CLAIMED } else { device_id }));
            let id = n;
            match phase {
                TouchPhase::Start => {
                    if self.pointer.is_none() && !claimed {
                        self.pointer = Some(id);
                        out.extend([Event::PointerMoved(pos), button(pos, true)]);
                    }
                }
                TouchPhase::Move => {
                    if self.pointer == Some(id) {
                        out.push(Event::PointerMoved(pos));
                    }
                }
                TouchPhase::End | TouchPhase::Cancel => {
                    self.down.remove(&id);
                    self.claimed.remove(&id);
                    if self.pointer == Some(id) {
                        self.pointer = None;
                        if phase == TouchPhase::End {
                            out.push(button(pos, false));
                        }
                        out.push(Event::PointerGone);
                    }
                }
            }
        }
        *events = out;
    }

    /// Forgets every finger (the window lost the focus: their lifts may never come); a pointer held lets go.
    fn forget(&mut self, events: &mut Vec<Event>) {
        if self.pointer.take().is_some() {
            events.push(Event::PointerGone);
        }
        self.down.clear();
        self.claimed.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{TouchDeviceId, TouchId, pos2};

    fn touch(id: u64, phase: TouchPhase, pos: Pos2) -> Event {
        Event::Touch {
            device_id: TouchDeviceId(1),
            id: TouchId(id),
            phase,
            pos,
            force: None,
        }
    }

    fn press(pos: Pos2, pressed: bool) -> Event {
        Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        }
    }

    /// The keyboard: everything under y = 400.
    fn keys(pos: Pos2) -> bool {
        pos.y > 400.0
    }

    /// The frame's pointer events after the route, in order.
    fn pointer(route: &mut TouchRoute, mut events: Vec<Event>) -> Vec<Event> {
        route.route(&mut events, Modifiers::NONE, keys);
        (events.into_iter())
            .filter(|e| !matches!(e, Event::Touch { .. }))
            .collect()
    }

    #[test]
    fn a_finger_on_the_keys_leaves_the_pointer_to_the_next_finger() {
        let mut route = TouchRoute::default();
        let (key, knob) = (pos2(100.0, 500.0), pos2(300.0, 100.0));
        // The integration presses the pointer with the first finger, on the keys: it goes.
        let first = vec![touch(0, TouchPhase::Start, key), Event::PointerMoved(key), press(key, true)];
        let mut events = first.clone();
        let mut probe = TouchRoute::default();
        probe.route(&mut events, Modifiers::NONE, keys);
        // The keys' finger is a touch of its own device: no pinch with a finger elsewhere.
        assert!(matches!(events[..], [Event::Touch { device_id: CLAIMED, .. }]));
        assert_eq!(pointer(&mut route, first), []);
        // The second finger, on the rack, takes the pointer.
        let second = vec![touch(1, TouchPhase::Start, knob)];
        assert_eq!(
            pointer(&mut route, second),
            [Event::PointerMoved(knob), press(knob, true)]
        );
        let to = pos2(300.0, 80.0);
        assert_eq!(
            pointer(&mut route, vec![touch(1, TouchPhase::Move, to), touch(0, TouchPhase::Move, key)]),
            [Event::PointerMoved(to)]
        );
        // The keys' finger lifts (the integration releases its pointer): the pointer stays down on the knob.
        let lift = vec![touch(0, TouchPhase::End, key), press(key, false), Event::PointerGone];
        assert_eq!(pointer(&mut route, lift), []);
        assert_eq!(
            pointer(&mut route, vec![touch(1, TouchPhase::End, to)]),
            [press(to, false), Event::PointerGone]
        );
        // No finger down: a mouse is left alone.
        let mouse = vec![Event::PointerMoved(knob), press(knob, true)];
        assert_eq!(pointer(&mut route, mouse.clone()), mouse);
    }

    #[test]
    fn with_no_finger_on_the_keys_the_pointer_follows_the_first_finger_as_before() {
        let mut route = TouchRoute::default();
        let (a, b) = (pos2(100.0, 100.0), pos2(300.0, 100.0));
        let first = vec![touch(0, TouchPhase::Start, a), Event::PointerMoved(a), press(a, true)];
        assert_eq!(
            pointer(&mut route, first),
            [Event::PointerMoved(a), press(a, true)]
        );
        // A second finger, on the keys or not, is a touch only.
        assert_eq!(pointer(&mut route, vec![touch(1, TouchPhase::Start, b)]), []);
        assert_eq!(pointer(&mut route, vec![touch(2, TouchPhase::Start, pos2(0.0, 500.0))]), []);
        assert_eq!(
            pointer(&mut route, vec![touch(0, TouchPhase::End, a)]),
            [press(a, false), Event::PointerGone]
        );
        // The finger still down does not take the pointer over; the next finger down does.
        assert_eq!(pointer(&mut route, vec![touch(1, TouchPhase::Move, b)]), []);
        let c = pos2(200.0, 200.0);
        assert_eq!(
            pointer(&mut route, vec![touch(3, TouchPhase::Start, c)]),
            [Event::PointerMoved(c), press(c, true)]
        );
    }

    #[test]
    fn a_lost_focus_or_a_reused_id_lets_the_pointer_go() {
        let mut route = TouchRoute::default();
        let a = pos2(100.0, 100.0);
        pointer(&mut route, vec![touch(0, TouchPhase::Start, a)]);
        // Its lift never came, and id 0 goes down again: the old press lets go, the new one presses.
        assert_eq!(
            pointer(&mut route, vec![touch(0, TouchPhase::Start, a)]),
            [press(a, false), Event::PointerGone, Event::PointerMoved(a), press(a, true)]
        );
        let lost = pointer(&mut route, vec![Event::WindowFocused(false)]);
        assert_eq!(lost, [Event::WindowFocused(false), Event::PointerGone]);
        let mouse = vec![Event::PointerMoved(a)];
        assert_eq!(pointer(&mut route, mouse.clone()), mouse);
    }
}
