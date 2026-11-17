//! The `Sample rate:` setting's fallback message (the PROJECT tab has no `Samplerate:` row now).

use super::*;

/// An output that runs at another rate than the `Sample rate:` setting's (the recorder stays at 48 kHz) says so once
/// for each pair of rates, and again only after they matched in between; `Device` (0) never does.
#[test]
fn an_output_that_falls_back_from_the_setting_says_so_once() {
    let fallback = |rig: &Rig| {
        (rig.app.nav.overlays.iter()).any(|o| {
            matches!(
                o,
                Overlay::Message {
                    tag: "sample_rate_fallback",
                    ..
                }
            )
        })
    };
    let mut rig = Rig::with_synth();
    rig.frame(vec![]);
    assert!(!fallback(&rig), "the output's own rate");
    rig.app.settings.sample_rate = 96_000;
    rig.frame(vec![]);
    assert!(fallback(&rig));
    rig.app.nav.overlays.clear();
    rig.frame(vec![]);
    assert!(!fallback(&rig), "once");
    rig.rec.0.borrow_mut().sample_rate = Some(96_000);
    rig.frame(vec![]);
    rig.rec.0.borrow_mut().sample_rate = None;
    rig.frame(vec![]);
    assert!(fallback(&rig), "again after the rates matched");
    rig.app.settings.sample_rate = 0;
    rig.frame(vec![]);
}
