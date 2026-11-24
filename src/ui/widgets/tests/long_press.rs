use super::*;

// The parameter menu opens on a long press of the mouse too (300 ms within 0.5 U); a knob's or slider's on the release of
// one held 500 ms that never slid over 2 points (`gesture::control_menu`): held, it drags.

impl Rig {
    /// Presses the control's centre with the mouse and holds it still, a frame every 1/60 s, until the frame `secs`
    /// after the press, which releases when `release`.
    fn hold(&mut self, secs: f64, release: bool) {
        let at = RECT.center();
        self.frame(vec![Event::PointerMoved(at)]);
        self.button(at, PointerButton::Primary, true);
        let start = self.time;
        while self.time + 1.0 / 60.0 < start + secs - 1e-9 {
            self.frame(vec![]);
        }
        self.time = start + secs - 1.0 / 60.0;
        if release {
            self.button(at, PointerButton::Primary, false);
        } else {
            self.frame(vec![]);
        }
    }
}

/// Toggles (bound: the `ParamMenu::Bound` items) and a bound button, whose menu opens at the long press.
fn rigs() -> Vec<Rig> {
    let mut rigs: Vec<Rig> = [Kind::Toggle]
        .map(|kind| Rig::new(kind, 0.25))
        .into();
    let mut button = Rig::new(Kind::Button, 0.0);
    button.style = |b| Button {
        menu: ParamMenu::Bound,
        ..b
    };
    rigs.push(button);
    rigs
}

const CONTROLS: [Kind; 3] = [Kind::Knob, Kind::VSlider, Kind::HSlider];

#[test]
fn a_mouse_held_still_300_ms_opens_the_parameter_menu_and_its_release_neither_clicks_nor_edits() {
    for mut rig in rigs() {
        let kind = rig.kind;
        rig.hold(crate::ui::gesture::LONG_PRESS_S, false);
        assert!(rig.popup_is_open(), "{kind:?}: open at its due");
        // Held on past egui's 0.8 s and moved: the press drags nothing under the menu.
        for _ in 0..40 {
            rig.frame(vec![]);
        }
        rig.frame(vec![Event::PointerMoved(RECT.center() + vec2(40.0, -40.0))]);
        rig.button(RECT.center(), PointerButton::Primary, false);
        assert!(!rig.clicked, "{kind:?}");
        assert_eq!(
            rig.value,
            if matches!(kind, Kind::Button) {
                0.0
            } else {
                0.25
            },
            "{kind:?}"
        );
        rig.frame(vec![]);
        assert!(rig.menu_rect().is_some(), "{kind:?}: still open");
        let texts: Vec<String> = texts(&rig.shapes).into_iter().map(|t| t.0).collect();
        assert!(
            texts.iter().any(|t| t == "Add automation track"),
            "{kind:?}"
        );
    }
}

#[test]
fn a_knob_or_slider_opens_its_menu_on_the_release_of_a_still_long_press_which_edits_nothing() {
    for kind in CONTROLS {
        let mut rig = Rig::new(kind, 0.25);
        rig.hold(crate::ui::gesture::CONTROL_LONG_PRESS_S + 0.2, false);
        assert!(!rig.popup_is_open(), "{kind:?}: no menu while held");
        rig.button(RECT.center(), PointerButton::Primary, false);
        assert!(!rig.clicked, "{kind:?}");
        assert_eq!(rig.value, 0.25, "{kind:?}");
        rig.frame(vec![]);
        assert!(rig.popup_is_open(), "{kind:?}: open on the release");
        assert!(rig.menu_rect().is_some(), "{kind:?}");
        let texts: Vec<String> = texts(&rig.shapes).into_iter().map(|t| t.0).collect();
        assert!(texts.iter().any(|t| t == "Reset"), "{kind:?}");
    }
}

#[test]
fn a_knob_or_slider_held_long_then_slid_changes_its_value_and_opens_no_menu() {
    for kind in CONTROLS {
        let mut rig = Rig::new(kind, 0.25);
        rig.hold(1.0, false);
        let mut at = RECT.center();
        for _ in 0..10 {
            at += vec2(4.0, -4.0);
            rig.frame(vec![Event::PointerMoved(at)]);
        }
        assert_ne!(rig.value, 0.25, "{kind:?}: it drags");
        rig.button(at, PointerButton::Primary, false);
        rig.frame(vec![]);
        assert!(!rig.popup_is_open(), "{kind:?}: no menu after a slide");
    }
}

#[test]
fn a_mouse_released_at_250_ms_is_a_click_and_opens_no_menu() {
    for mut rig in rigs() {
        let kind = rig.kind;
        rig.hold(0.25, true);
        rig.frame(vec![]);
        assert!(!rig.popup_is_open(), "{kind:?}");
        match kind {
            // A click flips a toggle (0.25 is on: off) and is reported by a button.
            Kind::Toggle => assert_eq!(rig.value, 0.0),
            Kind::Button => {
                rig.hold(0.25, true);
                assert!(rig.clicked);
            }
            _ => assert_eq!(rig.value, 0.25, "{kind:?}"),
        }
    }
}

#[test]
fn a_long_press_on_a_toggle_does_not_flip_it() {
    let mut rig = Rig::new(Kind::Toggle, 0.0);
    rig.hold(0.4, true);
    assert_eq!(rig.value, 0.0);
    assert!(rig.popup_is_open());
}

#[test]
fn a_knob_or_slider_held_300_ms_opens_no_menu_and_a_slide_before_its_500_ms_never_does() {
    for kind in [Kind::Knob, Kind::VSlider, Kind::HSlider] {
        let mut rig = Rig::new(kind, 0.25);
        rig.hold(0.4, false);
        assert!(!rig.popup_is_open(), "{kind:?}: closed at 400 ms");
        rig.button(RECT.center(), PointerButton::Primary, false);
        rig.frame(vec![]);
        // A finger resting on it, then sliding 3 points (short of egui's drag threshold) and held on: no menu.
        let mut rig = Rig::new(kind, 0.25);
        rig.hold(0.2, false);
        rig.frame(vec![Event::PointerMoved(RECT.center() + vec2(0.0, -3.0))]);
        for _ in 0..40 {
            rig.frame(vec![]);
        }
        assert!(!rig.popup_is_open(), "{kind:?}: a slide cancels it");
        rig.button(RECT.center() + vec2(0.0, -3.0), PointerButton::Primary, false);
        rig.frame(vec![]);
        assert!(!rig.popup_is_open(), "{kind:?}: nor opens it on the release");
    }
}
