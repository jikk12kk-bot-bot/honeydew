use eframe::egui::{Pos2, Rect, pos2};

use super::edit::*;
use super::*;
use crate::song::{Clip, Event, Lane, LaneKind, TimeSig};
use crate::ui::clips::TimeAxis;
use crate::ui::graph_lane::Prop;
use crate::ui::piano_roll::SnapMode;

/// A drum hit of `pad` at `pos` ticks, `ticks` long, velocity 100, pan 10.
fn drum(pad: u8, pos: u32, ticks: f64) -> Event {
    Event::Drum {
        pos,
        len_beats: ticks / BEAT,
        pad,
        velocity: 100,
        pan: 10,
        value: 7,
        repeat: 0,
    }
}

/// (pad, start, length) of every hit, in ticks.
fn hits(events: &[Event]) -> Vec<(u8, f64, f64)> {
    events.iter().filter_map(hit).collect()
}

fn clip(beats: f64) -> Clip {
    Lane::new(LaneKind::Drums).add_clip(0, beats).clone()
}

// The time axis.

#[test]
fn the_beat_is_u_d_over_4k_with_k_from_0_01_to_1_and_a_new_clip_at_4_u_per_beat() {
    let u = 32.0;
    // At d = 4: 1 U … 100 U per beat.
    assert_eq!(beat_w(u, 4, 1.0), 32.0);
    assert_eq!(beat_w(u, 4, 0.01), 3200.0);
    assert_eq!(beat_w(u, 4, 0.25), 128.0);
    // k is clamped to its range.
    assert_eq!(beat_w(u, 4, 2.0), 32.0);
    assert_eq!(beat_w(u, 4, 0.001), 3200.0);
    // A clip with no saved view: 4 U per beat, whatever d.
    for d in [2, 4, 8] {
        assert_eq!(beat_w(u, d, new_k(d)), 128.0, "d = {d}");
    }
    assert_eq!(new_k(4), 0.25);
}

#[test]
fn the_horizontal_scroll_range_is_twice_the_clip_length() {
    // A 4-beat clip at 128 points a beat: 8 beats of content, 5 of them in a 640-point grid.
    let max = |loop_beats: f64, grid_w: f32| {
        crate::ui::scroller::Axis {
            pos: 0.0,
            zoom: 1.0 / 128.0,
            range: [0.0, 2.0 * loop_beats],
            zoom_range: [1.0 / 128.0; 2],
            view: eframe::egui::Rangef::new(0.0, grid_w),
        }
        .max()
    };
    assert_eq!(max(4.0, 640.0), 3.0);
    assert_eq!(max(4.0, 2000.0), 0.0);
    assert_eq!(max(8.0, 640.0), 11.0);
}

#[test]
fn marks_are_4_px_bars_2_px_beats_1_px_finer_never_closer_than_0_6_u() {
    let u = 32.0;
    let ticks = |m: Vec<(f64, f32)>| {
        m.into_iter()
            .map(|(t, w)| (t as u32, w as u32))
            .collect::<Vec<_>>()
    };
    // 4 U a beat: quarter beats are 1 U apart, eighths 0.5 U (too close).
    let m = ticks(marks(0.0, 512.0, 128.0, u, TimeSig::default(), MARK_GAP));
    assert_eq!(m.len(), 17);
    assert_eq!(
        &m[..6],
        [(0, 4), (32, 1), (64, 1), (96, 1), (128, 2), (160, 1)]
    );
    assert_eq!(m[16], (512, 4));
    // 1 U a beat (k = 1): halves would be 0.5 U apart, so beats and bars only.
    assert_eq!(
        ticks(marks(0.0, 512.0, 32.0, u, TimeSig::default(), MARK_GAP)),
        [(0, 4), (128, 2), (256, 2), (384, 2), (512, 4)]
    );
    // 100 U a beat (k = 0.01): every tick (1/128 beat, 0.78 U).
    assert_eq!(
        ticks(marks(0.0, 4.0, 3200.0, u, TimeSig::default(), MARK_GAP)),
        [(0, 4), (1, 1), (2, 1), (3, 1), (4, 1)]
    );
    // Beats 0.5 U apart are not marked; every other one is, as on the ruler.
    assert_eq!(
        ticks(marks(0.0, 1024.0, 16.0, u, TimeSig::default(), MARK_GAP)),
        [(0, 4), (256, 2), (512, 4), (768, 2), (1024, 4)]
    );
    // Far out (0.05 U a beat, bars 0.2 U apart; issue #41) bars group as the ruler's marks do: every 4 bars (0.8 U),
    // 2 px every 16, 4 px on the labelled 32.
    let far = ticks(marks(0.0, 16384.0, 1.6, u, TimeSig::default(), MARK_GAP));
    assert_eq!(far.len(), 9);
    assert!(
        far.iter()
            .enumerate()
            .all(|(i, &(t, _))| t == i as u32 * 2048)
    );
    let widths: Vec<u32> = far.iter().map(|m| m.1).collect();
    assert_eq!(widths, [4, 1, 1, 1, 2, 1, 1, 1, 4]);
    // From the first mark at or after t0.
    assert_eq!(
        ticks(marks(100.0, 300.0, 128.0, u, TimeSig::default(), MARK_GAP)),
        [(128, 2), (160, 1), (192, 1), (224, 1), (256, 2), (288, 1)]
    );
}

#[test]
fn finer_marks_split_the_beat_by_the_subdivision_first() {
    // The timeline grid is 1/d beat (`TDIV` byte 0): coarser levels follow the ruler's Next, finer ones halve a step.
    let sig = |d| TimeSig {
        subdivision: d,
        beats_per_bar: 3,
    };
    let in_units = |m: Vec<(f64, f32)>, per_beat: f64| {
        (m.into_iter())
            .map(|(t, w)| ((t * per_beat / BEAT).round() as u32, w as u32))
            .collect::<Vec<_>>()
    };
    // d = 3, 2 U a beat: thirds are 0.67 U apart (sixths too close); a bar is 3 beats.
    let m = marks(0.0, 512.0, 64.0, 32.0, sig(3), MARK_GAP);
    assert!((m[1].0 - BEAT / 3.0).abs() < 1e-9);
    assert_eq!(
        in_units(m, 3.0),
        [
            (0, 4),
            (1, 1),
            (2, 1),
            (3, 2),
            (4, 1),
            (5, 1),
            (6, 2),
            (7, 1),
            (8, 1),
            (9, 4),
            (10, 1),
            (11, 1),
            (12, 2)
        ]
    );
    // d = 6, 4 U a beat: sixths 0.67 U apart.
    assert_eq!(
        in_units(marks(0.0, 128.0, 128.0, 32.0, sig(6), MARK_GAP), 6.0),
        [(0, 4), (1, 1), (2, 1), (3, 1), (4, 1), (5, 1), (6, 2)]
    );
    // d = 6, 2 U a beat: sixths are too close, so thirds, as the ruler's Next has it (d/n = 3).
    assert_eq!(
        in_units(marks(0.0, 128.0, 64.0, 32.0, sig(6), MARK_GAP), 3.0),
        [(0, 4), (1, 1), (2, 1), (3, 2)]
    );
}

#[test]
fn a_fixed_grid_size_draws_its_lines_and_auto_snaps_to_them_a_triplet_too() {
    // #53, at 2 U a beat (64 pt), where Auto is a step (32 ticks).
    use crate::ui::piano_roll::notes::{GridSize, Snap, auto_grid};
    let (u, sig) = (32.0, TimeSig::default());
    let auto = auto_grid(64.0, u, sig);
    let lines = |size, to, beat_w, sig| grid_lines(0.0, to, beat_w, u, sig, MARK_GAP, size);
    // In `unit`s of a tick, every line on one.
    let units = |m: Vec<(f64, f32)>, unit: f64| {
        (m.into_iter())
            .map(|(t, w)| {
                assert!(((t / unit).round() * unit - t).abs() < 1e-6, "{t}");
                ((t / unit).round() as u32, w as u32)
            })
            .collect::<Vec<_>>()
    };
    // 1/32 (16 ticks, 8 pt apart): its lines, 4 px on the bar and 2 on the beat; Auto snaps to it, Beat keeps a beat.
    let size = GridSize::Note(32, false);
    assert_eq!(
        units(lines(size, 128.0, 64.0, sig), 16.0),
        [
            (0, 4),
            (1, 1),
            (2, 1),
            (3, 1),
            (4, 1),
            (5, 1),
            (6, 1),
            (7, 1),
            (8, 2)
        ]
    );
    let snap = Snap {
        size,
        ..Snap::default()
    };
    assert_eq!(snap.active(auto, sig), Some(16.0));
    let beat = Snap {
        mode: SnapMode::Beat,
        ..snap
    };
    assert_eq!(beat.grid(auto, sig), 128.0);
    // 1/8T: a third of a beat (21.3 pt), the beats among its lines; Auto snaps to it.
    let size = GridSize::Note(8, true);
    assert_eq!(
        units(lines(size, 256.0, 64.0, sig), 128.0 / 3.0),
        [(0, 4), (1, 1), (2, 1), (3, 2), (4, 1), (5, 1), (6, 2)]
    );
    assert_eq!(Snap { size, ..snap }.grid(auto, sig), 128.0 / 3.0);
    // 1/64T lines 2.7 pt apart are too dense: every other one (1/32T, 5.3 pt) is drawn; Auto still snaps to 1/64T.
    let size = GridSize::Note(64, true);
    assert_eq!(units(lines(size, 128.0, 64.0, sig), 32.0 / 3.0).len(), 13);
    assert_eq!(Snap { size, ..snap }.grid(auto, sig), 16.0 / 3.0);
    // 1/1 (4 beats) in 3/4 at 16 pt a beat: the bar lines too.
    let three = TimeSig {
        subdivision: 4,
        beats_per_bar: 3,
    };
    assert_eq!(
        units(lines(GridSize::Note(1, false), 1536.0, 16.0, three), 128.0),
        [(0, 4), (3, 4), (4, 2), (6, 4), (8, 2), (9, 4), (12, 4)]
    );
    // Auto is the adaptive marks.
    assert_eq!(
        lines(GridSize::Auto, 512.0, 64.0, sig),
        marks(0.0, 512.0, 64.0, u, sig, MARK_GAP)
    );
}

// Hits.

#[test]
fn a_hit_is_its_pad_start_and_length_in_ticks() {
    assert_eq!(hit(&drum(2, 64, 64.0)), Some((2, 64.0, 64.0)));
    let note = Event::Note {
        pos: 0,
        len_beats: 1.0,
        key: 60,
        velocity: 1,
        pan: 1,
        slide: false,
    };
    assert_eq!(hit(&note), None);
}

#[test]
fn the_minimum_length_is_an_eighth_of_the_snap_unit_or_1_128_beat_with_snap_off() {
    assert_eq!(min_len(Some(32.0)), 4.0);
    assert_eq!(min_len(Some(128.0)), 16.0);
    assert_eq!(min_len(None), 1.0);
}

#[test]
fn a_new_hit_starts_on_the_nearest_snap_unit_with_the_remembered_values() {
    let r = Remembered {
        len_beats: 0.5,
        velocity: 90,
        pan: 30,
    };
    let start = |tick, snap| hit(&new_hit(1, tick, snap, &r)).map(|h| h.1);
    // The nearest unit, not the one below; never before 0.
    assert_eq!(start(47.0, Some(32.0)), Some(32.0));
    assert_eq!(start(49.0, Some(32.0)), Some(64.0));
    assert_eq!(start(-10.0, Some(32.0)), Some(0.0));
    // Snap off: the tap's own tick.
    assert_eq!(start(47.4, None), Some(47.0));
    assert_eq!(
        new_hit(1, 47.0, Some(32.0), &r),
        Event::Drum {
            pos: 32,
            len_beats: 0.5,
            pad: 1,
            velocity: 90,
            pan: 30,
            value: Event::DEFAULT_PITCH,
            repeat: 0,
        }
    );
    // The remembered length never goes below the minimum; the defaults are 1/4 beat, 255 and 127.
    let short = Remembered {
        len_beats: 0.001,
        ..r
    };
    assert_eq!(hit(&new_hit(0, 0.0, Some(32.0), &short)).unwrap().2, 4.0);
    assert_eq!(hit(&new_hit(0, 0.0, None, &short)).unwrap().2, 1.0);
    let d = Remembered::default();
    assert_eq!((d.len_beats, d.velocity, d.pan), (0.25, 255, 127));
}

#[test]
fn insert_keeps_the_clip_sorted_and_the_new_hit_unselected() {
    let mut events = vec![drum(0, 0, 32.0), drum(1, 32, 32.0), drum(2, 64, 32.0)];
    let mut sel = vec![true, false, true];
    assert_eq!(insert(&mut events, &mut sel, drum(3, 32, 8.0)), 2);
    assert_eq!(hits(&events)[2], (3, 32.0, 8.0));
    assert_eq!(sel, [true, false, false, true]);
}

#[test]
fn a_move_snaps_the_anchor_start_keeps_the_group_after_0_and_its_rows_in_range() {
    let orig = [drum(0, 0, 32.0), drum(1, 64, 32.0), drum(2, 128, 32.0)];
    let sel = [true, true, false];
    let which = [true, true, false];
    let mv = |anchor, dt, dr, snap| moved(&orig, &sel, &which, anchor, dt, dr, 4, snap);
    // 40 ticks on: the anchor's start goes to the nearest 32; a row down. The third hit stays.
    let (events, s) = mv(0, 40.0, 1, Some(32.0));
    assert_eq!(
        hits(&events),
        [(1, 32.0, 32.0), (2, 96.0, 32.0), (2, 128.0, 32.0)]
    );
    assert_eq!(s, [true, true, false]);
    // Re-sorted, the selection with it.
    let (events, s) = mv(0, 150.0, 0, Some(32.0));
    assert_eq!(
        hits(&events),
        [(2, 128.0, 32.0), (0, 160.0, 32.0), (1, 224.0, 32.0)]
    );
    assert_eq!(s, [false, true, true]);
    // Not before the clip start: the group's first hit stops at 0.
    assert_eq!(
        hits(&mv(1, -100.0, 0, Some(32.0)).0)[..2],
        [(0, 0.0, 32.0), (1, 64.0, 32.0)]
    );
    // Rows clamp to the existing ones, the group's shape kept.
    assert_eq!(
        hits(&mv(0, 0.0, 5, Some(32.0)).0)[..2],
        [(2, 0.0, 32.0), (3, 64.0, 32.0)]
    );
    assert_eq!(
        hits(&mv(0, 0.0, -3, Some(32.0)).0)[..2],
        [(0, 0.0, 32.0), (1, 64.0, 32.0)]
    );
    // Snap off: whole ticks; overlaps are allowed.
    assert_eq!(
        hits(&mv(0, 40.4, 0, None).0)[..2],
        [(0, 40.0, 32.0), (1, 104.0, 32.0)]
    );
    let (events, _) = mv(0, 128.0, 2, Some(32.0));
    assert_eq!(hits(&events)[..2], [(2, 128.0, 32.0), (2, 128.0, 32.0)]);
}

#[test]
fn the_end_capsule_moves_the_box_end_to_the_snapped_finger_and_every_length_by_as_much() {
    let orig = [drum(0, 0, 32.0), drum(1, 64, 64.0)];
    let len = |which: &[bool], finger, snap| -> Vec<f64> {
        hits(&resized(&orig, which, finger, snap))
            .iter()
            .map(|h| h.2)
            .collect()
    };
    // The box [0, 32]: the finger at 52 → 64.
    assert_eq!(len(&[true, false], 52.0, Some(32.0)), [64.0, 64.0]);
    // Both, the box [0, 128]: its end 150 → 160, each length + 32.
    assert_eq!(len(&[true, true], 150.0, Some(32.0)), [64.0, 96.0]);
    // The box's minimum width: 32/8, or a tick with snap off.
    assert_eq!(len(&[true, false], -8.0, Some(32.0)), [4.0, 64.0]);
    assert_eq!(len(&[true, false], -8.0, None), [1.0, 64.0]);
    assert_eq!(len(&[true, false], 35.4, None), [35.0, 64.0]);
    // Only the box has a minimum: the box [0, 128] down to 4 takes 124 off each, which keeps a tick.
    assert_eq!(len(&[true, true], 0.0, Some(32.0)), [1.0, 1.0]);
}

#[test]
fn copy_delete_and_snap_act_on_the_selection() {
    let mut events = vec![drum(0, 0, 32.0), drum(1, 96, 32.0), drum(2, 40, 8.0)];
    events.sort_by_key(Event::pos);
    // [0: pad 0 at 0, 1: pad 2 at 40, 2: pad 1 at 96]
    let mut sel = vec![true, false, true];
    // Span 0..128 → copies 128 on, which become the selection.
    copy_selected(&mut events, &mut sel, 32.0);
    assert_eq!(
        hits(&events),
        [
            (0, 0.0, 32.0),
            (2, 40.0, 8.0),
            (1, 96.0, 32.0),
            (0, 128.0, 32.0),
            (1, 224.0, 32.0)
        ]
    );
    assert_eq!(sel, [false, false, false, true, true]);
    // A lone zero-length hit copies one unit on.
    let mut one = vec![drum(0, 64, 0.0)];
    let mut s = vec![true];
    copy_selected(&mut one, &mut s, 32.0);
    assert_eq!(hits(&one), [(0, 64.0, 0.0), (0, 96.0, 0.0)]);
    // Snap: the nearest grid line.
    let mut sel = vec![false, true, false, false, false];
    quantize_selected(&mut events, &mut sel, 32.0);
    assert_eq!(hits(&events)[1], (2, 32.0, 8.0));
    let mut sel = vec![false, true, false, true, false];
    crate::ui::piano_roll::notes::remove_selected(&mut events, &mut sel);
    assert_eq!(
        hits(&events),
        [(0, 0.0, 32.0), (1, 96.0, 32.0), (1, 224.0, 32.0)]
    );
    assert_eq!(sel, [false; 3]);
}

#[test]
fn paste_puts_the_earliest_hit_on_the_tick_and_the_others_at_their_offsets_selected() {
    // As the automation editor's: pads and lengths as copied.
    let mut events = vec![drum(0, 0, 32.0)];
    let mut sel = vec![true];
    paste(
        &mut events,
        &mut sel,
        &[drum(1, 96, 16.0), drum(2, 40, 8.0)],
        200.0,
        None,
    );
    assert_eq!(
        hits(&events),
        [(0, 0.0, 32.0), (2, 200.0, 8.0), (1, 256.0, 16.0)]
    );
    assert_eq!(sel, [false, true, true]);
}

/// Row `pad`'s hits as (position in ticks, length in beats, velocity), in clip order.
fn fill_row(c: &Clip, pad: u8) -> Vec<(u32, f64, u8)> {
    (c.events.iter())
        .filter_map(|e| match *e {
            Event::Drum {
                pos,
                len_beats,
                velocity,
                pad: p,
                ..
            } if p == pad => Some((pos, len_beats, velocity)),
            _ => None,
        })
        .collect()
}

#[test]
fn fill_places_hits_every_n_steps_n_quarter_beats_long_at_velocity_255() {
    for every in [1usize, 2, 4] {
        let mut c = clip(4.0);
        c.events = vec![drum(0, 32, 32.0), drum(1, 33, 32.0), drum(1, 600, 32.0)];
        fill(&mut c, 1, every, TimeSig::default());
        let want: Vec<_> = (0..16)
            .step_by(every)
            .map(|s| (s as u32 * 32, every as f64 / 4.0, 255))
            .collect();
        assert_eq!(fill_row(&c, 1), want, "every {every}");
        // Other rows stay; every hit of the row goes first, past the loop too.
        assert!(hits(&c.events).contains(&(0, 32.0, 32.0)));
        assert!(!hits(&c.events).contains(&(1, 600.0, 32.0)));
    }
}

/// At d = TDIV byte 0 = 3 the hits are N/3 beat apart over llround(loop·3) subdivisions, and still N/4 beat long (not
/// N/3), so they overlap. A third of a beat falls between ticks: it rounds to the nearer.
#[test]
fn fill_at_3_steps_a_beat_spaces_hits_n_thirds_apart_n_quarter_beats_long() {
    let sig = |d| TimeSig {
        subdivision: d,
        beats_per_bar: 4,
    };
    let thirds = [0, 43, 85, 128, 171, 213, 256, 299, 341, 384, 427, 469];
    for every in [1usize, 2, 4] {
        let mut c = clip(4.0);
        c.events = vec![drum(1, 600, 32.0)];
        fill(&mut c, 1, every, sig(3));
        let want: Vec<_> = (thirds.iter().step_by(every))
            .map(|&pos| (pos, every as f64 / 4.0, 255))
            .collect();
        assert_eq!(fill_row(&c, 1), want, "every {every}");
    }
    // llround: 2.1 beats at d = 3 are 6.3 subdivisions, rounded to 6 (ceil would make 7).
    let mut c = clip(2.1);
    fill(&mut c, 0, 1, sig(3));
    assert_eq!(fill_row(&c, 0).len(), 6);
    // A d of 0 counts as 1 (as on the time axis): a hit a beat.
    let mut c = clip(4.0);
    fill(&mut c, 0, 1, sig(0));
    assert_eq!(
        fill_row(&c, 0),
        [
            (0, 0.25, 255),
            (128, 0.25, 255),
            (256, 0.25, 255),
            (384, 0.25, 255)
        ]
    );
}

/// The free grid fills the whole loop: the step grid's 512-step cap is not its. At d = 8 an 80-beat loop is 640
/// subdivisions, the last at 639/8 beat.
#[test]
fn fill_covers_a_free_grid_loop_past_the_step_grids_512_steps() {
    let sig = TimeSig {
        subdivision: 8,
        beats_per_bar: 4,
    };
    let mut c = clip(80.0);
    fill(&mut c, 0, 1, sig);
    let row = fill_row(&c, 0);
    assert_eq!(row.len(), 640);
    assert_eq!(row.last(), Some(&(10_224, 0.25, 255)));
}

#[test]
fn the_loop_fits_the_hits_ends_in_power_of_two_bars() {
    let mut c = clip(4.0);
    // A hit's end counts: 511 + 64 needs a second bar.
    c.events = vec![drum(0, 511, 64.0)];
    c.fit_loop(4);
    assert_eq!((c.loop_len_beats, c.visible_len_beats), (8.0, 0.0));
    c.events = vec![drum(0, 511, 0.0), drum(0, 512, 0.0)];
    c.fit_loop(4);
    assert_eq!((c.loop_len_beats, c.visible_len_beats), (8.0, 0.0));
    // Bars double (16, not 12). A new clip (visible length 0) shows its loop.
    c.events.push(drum(0, 1100, 0.0));
    c.fit_loop(4);
    assert_eq!((c.loop_len_beats, c.visible_len_beats), (16.0, 0.0));
    assert_eq!(c.visible_ticks(), 16.0 * 128.0);
    // It shrinks back to one bar (the generic rule), the length with it.
    c.events.clear();
    c.fit_loop(4);
    assert_eq!((c.loop_len_beats, c.visible_len_beats), (4.0, 0.0));
    assert_eq!(c.visible_ticks(), 4.0 * 128.0);
    // A resized clip's length stays, one equal to the loop included.
    c.visible_len_beats = 4.0;
    c.events.push(drum(0, 1100, 0.0));
    c.fit_loop(4);
    assert_eq!((c.loop_len_beats, c.visible_len_beats), (16.0, 4.0));
    c.events.clear();
    c.fit_loop(4);
    assert_eq!((c.loop_len_beats, c.visible_len_beats), (4.0, 4.0));
}

#[test]
fn the_graph_paint_sets_the_rows_eligible_hits_whose_extent_holds_the_tick() {
    let orig = vec![drum(0, 0, 32.0), drum(0, 64, 0.0), drum(1, 0, 32.0)];
    let vel = |e: &[Event]| {
        e.iter()
            .map(|e| Prop::Velocity.get(e).unwrap())
            .collect::<Vec<_>>()
    };
    let mut events = orig.clone();
    assert!(paint(&mut events, &[false; 3], 0, Prop::Velocity, 16.0, 50));
    assert_eq!(vel(&events), [50, 100, 100]);
    // The end is in the extent; the same value again changes nothing.
    assert!(!paint(
        &mut events,
        &[false; 3],
        0,
        Prop::Velocity,
        32.0,
        50
    ));
    // A zero-length hit at exactly its start.
    assert!(paint(&mut events, &[false; 3], 0, Prop::Velocity, 64.0, 9));
    assert!(!paint(&mut events, &[false; 3], 0, Prop::Velocity, 64.5, 1));
    assert_eq!(vel(&events), [50, 9, 100]);
    // While any is selected, only the selected.
    let mut events = orig.clone();
    assert!(!paint(
        &mut events,
        &[false, true, false],
        0,
        Prop::Pan,
        16.0,
        0
    ));
    assert!(paint(
        &mut events,
        &[false, true, false],
        0,
        Prop::Pan,
        64.0,
        0
    ));
}

// Geometry.

/// Tick 0 at x 100, 128 points a beat; rows 64 points from y 48; four rows.
const PLACE: Place = Place {
    axis: TimeAxis {
        origin: 100.0,
        beat_w: 128.0,
    },
    row_top: 48.0,
    pitch: 64.0,
    rows: 4,
    u: 32.0,
};

#[test]
fn a_hits_extent_is_its_start_to_its_end_on_its_row() {
    assert_eq!(
        PLACE.rect(&drum(1, 64, 32.0)),
        Some(Rect::from_min_max(pos2(164.0, 112.0), pos2(196.0, 176.0)))
    );
    assert_eq!(PLACE.row(47.0), None);
    assert_eq!(PLACE.row(48.0), Some(0));
    assert_eq!(PLACE.row(303.0), Some(3));
    assert_eq!(PLACE.row(304.0), None);
}

#[test]
fn hit_testing_uses_the_extent_with_no_minimum_width_topmost_first() {
    let events = [drum(1, 64, 32.0), drum(1, 128, 0.0), drum(1, 80, 32.0)];
    let at = |x: f32| PLACE.hit_at(&events, Pos2::new(x, 140.0));
    assert_eq!(at(165.0), Some(0));
    // Overlapping: the later one, drawn on top.
    assert_eq!(at(190.0), Some(2));
    assert_eq!(at(212.0), Some(2));
    assert_eq!(at(212.5), None);
    // A zero-length hit only at its start.
    assert_eq!(at(228.0), Some(1));
    assert_eq!(at(228.5), None);
    // Another row.
    assert_eq!(PLACE.hit_at(&events, pos2(165.0, 100.0)), None);
}

#[test]
fn the_marquee_selects_hits_whose_extent_overlaps_it() {
    let events = [drum(1, 64, 32.0), drum(1, 128, 0.0), drum(3, 0, 32.0)];
    let marquee = |a: Pos2, b: Pos2| PLACE.in_box(&events, Rect::from_two_pos(a, b));
    assert_eq!(
        marquee(pos2(150.0, 100.0), pos2(170.0, 120.0)),
        [true, false, false]
    );
    assert_eq!(
        marquee(pos2(220.0, 130.0), pos2(230.0, 130.0)),
        [false, true, false]
    );
    assert_eq!(marquee(pos2(0.0, 0.0), pos2(400.0, 400.0)), [true; 3]);
    assert_eq!(marquee(pos2(197.0, 130.0), pos2(227.0, 300.0)), [false; 3]);
}

#[test]
fn the_pop_in_scales_from_nothing_past_full_size_and_settles() {
    // a ← 0.6·a per 60 Hz frame; the scale 1.0515·sin(1.885·(1 − a)).
    assert_eq!(pop_scale(1.0), 0.0);
    let frame = 1.0 / 60.0;
    let scale = |frames: f64| pop_scale(pop_in(POP_RATE, frames * frame));
    assert!((scale(1.0) - 0.7199).abs() < 1e-3, "{}", scale(1.0));
    assert!(scale(3.0) > 1.04, "it overshoots: {}", scale(3.0));
    // 0.6^9 is just over 0.01, 0.6^10 under: over, full size.
    assert!(pop_in(POP_RATE, 9.0 * frame) > 0.0);
    assert_eq!(pop_in(POP_RATE, 10.0 * frame), 0.0);
    assert_eq!(scale(10.0), 1.0);
}

// Offscreen egui: the view in a real context, driven by synthetic input. Each action is applied to the song.

use eframe::egui::{self, Color32, Context, Modifiers, PointerButton, RawInput, Vec2, vec2};

use crate::metrics::{DEFAULT_UI_SCALE, Metrics};
use crate::song::{Channel, ChannelKind, DrumGrid, Song};
use crate::theme::{channel_color, theme_by_id};
use crate::ui::UiCtx;
use crate::ui::step_seq::{DrumPads, StepAction, hit_color, hit_tints};

/// A fake kit of four pads.
struct Kit;

impl DrumPads for Kit {
    fn count(&self) -> usize {
        4
    }

    fn names(&self, pad: usize) -> (&str, &str) {
        [
            ("Kick", "808 Kick"),
            ("Snare", "808 Snare"),
            ("Hat", "808 Hat"),
            ("Clap", "808 Clap"),
        ][pad]
    }

    fn midi_note(&self, _: usize) -> Option<u8> {
        None
    }

    fn custom_name(&self, _: usize) -> &str {
        ""
    }
}

const SCREEN: Rect = Rect {
    min: pos2(0.0, 0.0),
    max: pos2(1170.0, 540.0),
};
/// The channel under test, and the left of its grid (the 3 U header column and the t gap, ceil(U/16) px).
const CH: usize = 1;
const GRID_X: f32 = 98.0;
/// The rows' top at U = 32 (ppp 1): the ruler's height; row 0 starts on the nearest pixel, [`Y0`].
const TOP: f32 = 52.0;

/// Row `pad`'s centre line at `tick`, at the default view: 4 U (128 points) a beat is a point a tick from the grid's
/// left; rows 2 U (64 points) from below the ruler.
fn at(pad: usize, tick: f32) -> Pos2 {
    pos2(GRID_X + tick, TOP + 64.0 * pad as f32 + 32.0)
}

/// [`notes_lane`] with no hits in the one-bar clip at 0 a playlist `+Clip` makes: the free grid makes no clip itself.
fn empty_clip() -> Channel {
    let mut channel = notes_lane(&[]);
    channel.lanes[0].add_clip(0, 4.0);
    channel
}

/// A drums channel whose lane 0 is a notes lane holding `hits` in a one-bar clip (no clip without hits).
fn notes_lane(hits: &[Event]) -> Channel {
    let mut song = Song::default();
    song.add_channel(ChannelKind::Drums);
    let mut channel = song.channels.remove(1);
    channel.lanes[0].grid = DrumGrid::Notes;
    if !hits.is_empty() {
        channel.lanes[0].add_clip(0, 4.0).events = hits.to_vec();
    }
    channel
}

struct Rig {
    ctx: Context,
    view: DrumGridView,
    song: Song,
    /// The clip the view edits, by id: the lane's first, as the app opens it; none without one.
    clip: Option<u32>,
    remembered: Remembered,
    ppp: f32,
    time: f64,
    actions: Vec<StepAction>,
    shapes: Vec<egui::epaint::ClippedShape>,
}

impl Rig {
    fn new(channel: Channel) -> Self {
        Self::at_ppp(channel, 1.0)
    }

    fn at_ppp(channel: Channel, ppp: f32) -> Self {
        let clip = channel
            .lanes
            .first()
            .and_then(|l| l.clips.first())
            .map(|c| c.id);
        let mut song = Song::default();
        song.channels.push(channel);
        let mut rig = Self {
            ctx: Context::default(),
            view: DrumGridView::default(),
            song,
            clip,
            remembered: Remembered::default(),
            ppp,
            time: 0.0,
            actions: Vec::new(),
            shapes: Vec::new(),
        };
        for _ in 0..3 {
            rig.frame(vec![]);
        }
        rig.actions.clear();
        rig
    }

    fn frame(&mut self, events: Vec<egui::Event>) {
        self.frame_after(0.1, events);
    }

    fn frame_after(&mut self, dt: f64, events: Vec<egui::Event>) {
        self.time += dt;
        let mut raw = RawInput {
            screen_rect: Some(SCREEN),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let viewport = raw.viewports.entry(raw.viewport_id).or_default();
        viewport.native_pixels_per_point = Some(self.ppp);
        let (theme, m) = (
            theme_by_id(0),
            Metrics::from_pixels_per_point(self.ppp, DEFAULT_UI_SCALE),
        );
        let cx = UiCtx {
            theme: &theme,
            m: &m,
        };
        let (view, song, clip, remembered) =
            (&mut self.view, &self.song, self.clip, &mut self.remembered);
        let mut action = None;
        let output = self.ctx.run(raw, |ctx| {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ctx, |ui| {
                    action = view.show(ui, &cx, song, CH, (0, clip), &Kit, remembered);
                    view.graph_knob(ui, &cx, SCREEN, egui::Color32::WHITE, &mut 0.0);
                });
        });
        self.shapes = output.shapes;
        if let Some(action) = action {
            self.view.apply(&mut self.song, CH, (0, self.clip), &action);
            self.actions.push(action);
        }
    }

    fn button(&mut self, pos: Pos2, button: PointerButton, pressed: bool) {
        self.frame(vec![egui::Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: Modifiers::NONE,
        }]);
    }

    fn click_with(&mut self, pos: Pos2, button: PointerButton) {
        self.frame(vec![egui::Event::PointerMoved(pos)]);
        self.button(pos, button, true);
        self.button(pos, button, false);
        // Lets a popup or the floating row lay itself out.
        self.frame(vec![]);
    }

    fn click(&mut self, pos: Pos2) {
        self.click_with(pos, PointerButton::Primary);
    }

    /// Presses at `from`, moves through `offsets` (from the press), one frame each, and releases.
    fn drag(&mut self, from: Pos2, offsets: &[Vec2]) {
        self.frame(vec![egui::Event::PointerMoved(from)]);
        self.button(from, PointerButton::Primary, true);
        for &d in offsets {
            self.frame(vec![egui::Event::PointerMoved(from + d)]);
        }
        let end = from + offsets.last().copied().unwrap_or_default();
        self.button(end, PointerButton::Primary, false);
        self.frame(vec![]);
    }

    fn release(&mut self, pos: Pos2) {
        self.button(pos, PointerButton::Primary, false);
        self.frame(vec![]);
    }

    fn events(&self) -> &[Event] {
        let lane = &self.song.channels[CH].lanes[0];
        lane.clips.first().map_or(&[], |c| &c.events)
    }

    fn hits(&self) -> Vec<(u8, f64, f64)> {
        hits(self.events())
    }

    fn text_at(&self, label: &str) -> Option<Pos2> {
        self.shapes.iter().find_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.text() == label => {
                Some(t.visual_bounding_rect().center())
            }
            _ => None,
        })
    }

    fn rects(&self, color: Color32) -> Vec<Rect> {
        (self.shapes.iter())
            .filter_map(|s| match &s.shape {
                egui::Shape::Rect(r) if r.fill == color => Some(r.rect),
                _ => None,
            })
            .collect()
    }

    /// The hit colour on page `prop` of a hit at `value`.
    fn hit_color(&self, prop: Prop, value: u16, selected: bool) -> Color32 {
        let theme = theme_by_id(0);
        let color = channel_color(self.song.channels[CH].hue, &theme);
        hit_color(
            &hit_tints(&theme.colors, color, prop),
            prop,
            value,
            selected,
        )
    }

    /// The `Hits` edits, as their final flags.
    fn finals(&self) -> Vec<bool> {
        (self.actions.iter())
            .filter_map(|a| match a {
                StepAction::Hits { final_edit, .. } => Some(*final_edit),
                _ => None,
            })
            .collect()
    }
}

#[test]
fn the_view_edits_its_clip_by_id_and_a_gone_id_edits_nothing() {
    let mut channel = notes_lane(&[drum(0, 0, 32.0)]);
    channel.lanes[0].add_clip(512, 4.0);
    let mut rig = Rig::new(channel);
    // Clip 2, the lane's second: the tap lands there.
    rig.clip = Some(2);
    rig.frame(vec![]);
    rig.click(at(1, 47.0));
    let counts = |rig: &Rig| -> Vec<usize> {
        (rig.song.channels[CH].lanes[0].clips.iter())
            .map(|c| c.events.len())
            .collect()
    };
    assert_eq!(counts(&rig), [1, 1]);
    // An id that is gone: no other clip takes its place.
    rig.clip = Some(9);
    rig.frame(vec![]);
    rig.click(at(2, 47.0));
    assert_eq!(counts(&rig), [1, 1]);
}

#[test]
fn a_lane_with_no_clip_takes_no_hit_and_makes_no_clip() {
    // The playlist's `+Clip` makes clips; the grid edits the one it is on.
    let mut rig = Rig::new(notes_lane(&[]));
    rig.click(at(1, 47.0));
    assert!(rig.song.channels[CH].lanes[0].clips.is_empty());
}

#[test]
fn a_tapped_hit_pops_in_from_its_centre() {
    let mut rig = Rig::new(empty_clip());
    rig.click(at(1, 47.0));
    // The tap's frame and the next: the click helper's extra frame showed it first, at a = 1, nothing.
    let fill = rig.hit_color(Prop::Velocity, 255, false);
    let size = |rig: &Rig| rig.rects(fill).first().map(|r| (r.center().x, r.width()));
    assert_eq!(size(&rig), Some((GRID_X + 48.0, 0.0)));
    rig.frame_after(1.0 / 60.0, vec![]);
    let (x, w) = size(&rig).unwrap();
    assert_eq!(x, GRID_X + 48.0);
    assert!((w - 32.0 * 0.7199).abs() < 0.05, "{w}");
    rig.frame(vec![]);
    rig.frame(vec![]);
    assert_eq!(size(&rig), Some((GRID_X + 48.0, 32.0)));
}

#[test]
fn the_graph_lane_lists_three_pages_and_bars_span_each_hit_of_the_current_row() {
    let hits = [drum(0, 64, 64.0), drum(0, 192, 0.0), drum(1, 0, 32.0)];
    let mut rig = Rig::new(notes_lane(&hits));
    rig.view.graph.open = true;
    rig.frame(vec![]);
    for label in ["Velocity", "Panning", "Pitch"] {
        assert!(rig.text_at(label).is_some(), "{label}");
    }
    assert_eq!(rig.text_at("Repeat"), None);
    // The lane is 270 points tall from y 270 (f 0.5 of the 540-point view, whose top margin is 0); its value area starts
    // 4 px lower, right of the header column.
    let fill = rig.hit_color(Prop::Velocity, 100, false);
    let bar = |x0: f32, x1: f32| {
        let top = (540.0 - 266.0 * 100.0 / 255.0_f32).round();
        Rect::from_min_max(pos2(x0, top), pos2(x1, 540.0))
    };
    let bars: Vec<Rect> = rig
        .rects(fill)
        .into_iter()
        .filter(|r| r.top() > 270.0)
        .collect();
    // The Kick's two: over its 64 ticks, and a pixel for the zero-length one. Not the Snare's.
    let x = |tick: f32| GRID_X + tick;
    assert_eq!(bars, [bar(x(64.0), x(128.0)), bar(x(192.0), x(193.0))]);
    // A paint across both at mid-height sets 127, in one undo step, and remembers it.
    rig.drag(pos2(x(74.0), 407.0), &[vec2(30.0, 0.0), vec2(118.0, 0.0)]);
    let vel: Vec<_> = rig
        .events()
        .iter()
        .map(|e| Prop::Velocity.get(e).unwrap())
        .collect();
    assert_eq!(vel, [127, 127, 100]);
    assert_eq!(rig.finals().last(), Some(&true));
    assert_eq!(rig.finals().iter().filter(|f| **f).count(), 1);
    assert_eq!(rig.remembered.velocity, 127);
}

#[test]
fn the_graph_lanes_column_starting_on_the_loop_end_is_still_in_range() {
    // The in-range test is start ≤ L, as the step view's. A one-beat loop: step 4 starts on L, in beat 1 (odd):
    // `moduleFill`; step 5 is past it: `controlFill`.
    let mut channel = notes_lane(&[drum(0, 0, 32.0)]);
    channel.lanes[0].clips[0].loop_len_beats = 1.0;
    let mut rig = Rig::new(channel);
    rig.view.graph.open = true;
    rig.frame(vec![]);
    let colors = theme_by_id(0).colors;
    let left = rig.view.last.unwrap().grid_left();
    let column = |rig: &Rig, step: f32, fill: egui::Color32| {
        (rig.rects(fill).iter()).any(|r| r.top() > 270.0 && r.left() == left + 32.0 * step)
    };
    assert!(column(&rig, 4.0, colors.module_fill.to_color32()));
    assert!(column(&rig, 5.0, colors.control_fill.to_color32()));
}

#[test]
fn the_playhead_line_is_cut_at_the_header_columns_right_edge_and_at_beat_0_shows_in_the_t_gap() {
    // Drawn last with the rows' painter, its t + 1 px would spill over the row headers. As the step view's: cut at the
    // header column's right edge, its left half in the t gap.
    let mut rig = Rig::new(notes_lane(&[drum(0, 0, 32.0)]));
    rig.view.playhead = 0;
    rig.frame(vec![]);
    let (text, shadow) = {
        let c = theme_by_id(0).colors;
        let shadow = c.background.to_color32().gamma_multiply(0.3);
        (c.text.to_color32(), shadow)
    };
    let column = 3.0 * 32.0;
    let lines: Vec<(Rect, Rect)> = (rig.shapes.iter())
        .filter_map(|s| match &s.shape {
            egui::Shape::Rect(r) if [text, shadow].contains(&r.fill) && r.rect.top() == TOP => {
                Some((r.rect, s.clip_rect))
            }
            _ => None,
        })
        .collect();
    // The shadow and the line, centred on the grid's left (t right of the column), cut at the column.
    assert_eq!(
        lines
            .iter()
            .map(|(r, c)| (crate::ui::timeline::testing::line_x(*r, 1.0), c.left()))
            .collect::<Vec<_>>(),
        [(column + 2.0, column); 2]
    );
}

#[test]
fn the_lanes_knob_drags_f_on_the_editor_below_its_top_margin() {
    // f = (bottom − y)/H, H the whole view (its top margin is 0); the press is in the tab over the lane's top.
    let mut rig = Rig::new(notes_lane(&[drum(0, 0, 32.0)]));
    rig.view.graph.open = true;
    rig.frame(vec![]);
    let knob = pos2(44.8, 270.0);
    rig.frame(vec![egui::Event::PointerMoved(knob)]);
    rig.button(knob, PointerButton::Primary, true);
    for y in [250.0, 230.0, 216.0] {
        rig.frame(vec![egui::Event::PointerMoved(pos2(knob.x, y))]);
    }
    rig.button(pos2(knob.x, 216.0), PointerButton::Primary, false);
    assert!(
        (rig.view.graph.f - 0.6).abs() < 1e-6,
        "{}",
        rig.view.graph.f
    );
}

impl Rig {
    /// A finger: the touch event plus the pointer events an integration sends with it.
    fn touch(&mut self, phase: egui::TouchPhase, pos: Pos2) {
        let touch = egui::Event::Touch {
            device_id: egui::TouchDeviceId(1),
            id: egui::TouchId(1),
            phase,
            pos,
            force: None,
        };
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        let pointer = match phase {
            egui::TouchPhase::Start => vec![egui::Event::PointerMoved(pos), button(true)],
            egui::TouchPhase::Move => vec![egui::Event::PointerMoved(pos)],
            _ => vec![button(false), egui::Event::PointerGone],
        };
        self.frame([vec![touch], pointer].concat());
    }

    /// A second finger: its touch event only, as an integration sends for every finger but the first.
    fn second(&mut self, phase: egui::TouchPhase, pos: Pos2) {
        self.frame(vec![egui::Event::Touch {
            device_id: egui::TouchDeviceId(1),
            id: egui::TouchId(2),
            phase,
            pos,
            force: None,
        }]);
    }

    fn loop_beats(&self) -> f64 {
        self.song.channels[CH].lanes[0].clips[0].loop_len_beats
    }
}

#[test]
fn a_subdivision_of_0_counts_as_1_and_draws_no_marks_at_no_width() {
    // `TDIV` is stored unchecked, so a file may hold 0: a beat as at d = 1, never 0 wide.
    assert_eq!(beat_w(32.0, 0, 1.0), beat_w(32.0, 1, 1.0));
    assert_eq!(beat_w(32.0, 0, 1.0), 8.0);
    assert_eq!(new_k(0), new_k(1));
    // No width, no marks (and no endless loop to an infinite tick).
    assert_eq!(
        marks(0.0, 512.0, 0.0, 32.0, TimeSig::default(), MARK_GAP),
        []
    );
    assert_eq!(
        marks(
            0.0,
            f64::INFINITY,
            f32::NAN,
            32.0,
            TimeSig::default(),
            MARK_GAP
        ),
        []
    );
}

#[test]
fn the_beat_width_follows_tdiv_byte_0_and_the_bar_lines_byte_1() {
    // Beat width = U·d/(4k), d = TDIV byte 0; a new clip opens at 4 U a beat.
    for (d, b) in [(3u8, 3u8), (6, 7)] {
        let mut rig = Rig::new(notes_lane(&[]));
        rig.song.time_sig = TimeSig {
            subdivision: d,
            beats_per_bar: b,
        };
        rig.view.views.clear();
        rig.frame(vec![]);
        assert_eq!(
            rig.view.views[&(CH, 0, None)].k,
            f32::from(d) / 16.0,
            "d {d}"
        );
        // 4 px bar lines (at 1 px a point) every b beats: 128 points a beat from the grid's left. On the grid, under
        // the ruler (whose beat ticks are 2t = 4 points wide).
        let bars: Vec<f32> = (rig.shapes.iter())
            .filter_map(|s| match &s.shape {
                egui::Shape::Rect(r)
                    if (r.rect.width() - 4.0).abs() < 1e-3 && r.rect.top() >= 48.0 =>
                {
                    Some(r.rect.center().x)
                }
                _ => None,
            })
            .filter(|&x| x > GRID_X - 1.0 && x < GRID_X + 1000.0)
            .collect();
        let want = |n: f32| GRID_X + n * f32::from(b) * 128.0;
        assert!(
            bars.iter().any(|&x| (x - want(1.0)).abs() < 0.5),
            "b {b}: {bars:?}"
        );
        assert!(
            !bars.iter().any(|&x| (x - GRID_X - 512.0).abs() < 0.5),
            "b {b}: {bars:?}"
        );
    }
}

#[test]
fn fill_and_clear_fit_the_loop() {
    // The loop is fitted after every clip edit, the row menu's too.
    let mut rig = Rig::new(notes_lane(&[drum(0, 0, 32.0), drum(1, 600, 32.0)]));
    rig.song.channels[CH].lanes[0].clips[0].loop_len_beats = 8.0;
    let clip = rig.clip;
    rig.view
        .apply(&mut rig.song, CH, (0, clip), &StepAction::Clear { pad: 1 });
    assert_eq!(rig.song.channels[CH].lanes[0].clips[0].loop_len_beats, 4.0);
    // Fill over a three-bar loop: its quarter-note hits reach 12 beats, which fit in 16.
    rig.song.channels[CH].lanes[0].clips[0].loop_len_beats = 12.0;
    let fill = StepAction::Fill { pad: 0, every: 4 };
    rig.view.apply(&mut rig.song, CH, (0, clip), &fill);
    assert_eq!(rig.song.channels[CH].lanes[0].clips[0].loop_len_beats, 16.0);
}

/// The row menu's Fill reads d from TDIV byte 0, not byte 1, the beats per bar.
#[test]
fn the_row_menus_fill_spaces_hits_by_tdiv_byte_0() {
    let mut rig = Rig::new(notes_lane(&[drum(0, 0, 32.0)]));
    rig.song.time_sig = TimeSig {
        subdivision: 3,
        beats_per_bar: 4,
    };
    let clip = rig.clip;
    let fill = StepAction::Fill { pad: 0, every: 4 };
    rig.view.apply(&mut rig.song, CH, (0, clip), &fill);
    assert_eq!(
        rig.hits(),
        [(0, 0.0, 128.0), (0, 171.0, 128.0), (0, 341.0, 128.0)]
    );
}

/// The other gestures the remembered-hits guard covers: a graph paint whose release, pinch, or closed graph lane leaves
/// the hits as they were does not fit the loop; one that changed them does. A release or pinch that leaves them is a
/// touch that changed nothing ([`StepAction::GraphCancel`], whose own undo re-syncs).
#[test]
fn a_graph_paint_that_puts_the_values_back_leaves_a_long_loop_as_it_was() {
    // Four bars holding one Kick hit in bar 1, the graph lane open on Velocity: y 435 paints the hit's own 100,
    // y 407 paints 127, at x 110 (tick 12, inside the hit).
    let long = || {
        let mut rig = Rig::new(notes_lane(&[drum(0, 0, 32.0)]));
        rig.song.channels[CH].lanes[0].clips[0].loop_len_beats = 16.0;
        rig.view.graph.open = true;
        rig.frame(vec![]);
        rig
    };
    let (own, mid) = (pos2(110.0, 435.0), pos2(110.0, 407.0));
    let velocity = |rig: &Rig| Prop::Velocity.get(&rig.events()[0]);
    let unchanged = |rig: &Rig| matches!(rig.actions.last(), Some(StepAction::GraphCancel(_)));
    // Released after painting back.
    let mut rig = long();
    rig.drag(own, &[mid - own, Vec2::ZERO]);
    assert_eq!((velocity(&rig), rig.loop_beats()), (Some(100), 16.0));
    assert_eq!(rig.finals(), [false, false]);
    assert!(unchanged(&rig));
    // A second finger ends the paint, putting back what it painted.
    let mut rig = long();
    use egui::TouchPhase::*;
    rig.touch(Start, own);
    rig.touch(Move, mid);
    assert_eq!(velocity(&rig), Some(127));
    rig.second(Start, at(2, 300.0));
    rig.second(End, at(2, 300.0));
    rig.touch(End, mid);
    assert_eq!((velocity(&rig), rig.loop_beats()), (Some(100), 16.0));
    assert!(unchanged(&rig));
    // The graph lane closes during a paint that went back.
    let mut rig = long();
    rig.frame(vec![egui::Event::PointerMoved(own)]);
    rig.button(own, PointerButton::Primary, true);
    rig.frame(vec![egui::Event::PointerMoved(mid)]);
    rig.frame(vec![egui::Event::PointerMoved(own)]);
    rig.view.graph.open = false;
    rig.frame(vec![]);
    assert_eq!((velocity(&rig), rig.loop_beats()), (Some(100), 16.0));
    assert_eq!(rig.finals().last(), Some(&true));
    rig.release(own);
    // A paint that changed the velocity fits the loop: one bar holds the hit.
    let mut rig = long();
    rig.drag(own, &[mid - own]);
    assert_eq!((velocity(&rig), rig.loop_beats()), (Some(127), 4.0));
    // A pinch before any paint (the touch-down at the hit's own value): a touch that changed nothing, once.
    let mut rig = long();
    rig.touch(Start, own);
    rig.second(Start, at(2, 300.0));
    assert!(unchanged(&rig));
    rig.second(End, at(2, 300.0));
    rig.touch(End, own);
    let cancels = rig
        .actions
        .iter()
        .filter(|a| matches!(a, StepAction::GraphCancel(_)));
    assert_eq!((cancels.count(), rig.loop_beats()), (1, 16.0));
}

#[test]
fn a_trackpad_pinch_over_the_free_grid_zooms_the_time_only() {
    // The playlist's pinch zone, zoom x.
    let mut rig = Rig::new(notes_lane(&[drum(0, 0, 32.0)]));
    let view = |rig: &Rig| *rig.view.views.values().next().unwrap();
    let before = view(&rig);
    rig.frame(vec![
        egui::Event::PointerMoved(pos2(600.0, 150.0)),
        egui::Event::Zoom(2.0),
    ]);
    let after = view(&rig);
    assert!((after.k - before.k / 2.0).abs() < 1e-6, "{after:?}");
    assert_eq!(after.rows, before.rows);
}

#[test]
fn the_drum_editors_zones_are_round_21s() {
    // An 800 × 448 editor, the drum mixer half out (HW = 3 U + 0.5·12 U = 288 points, GL = 288 + ceil(U/16) = 290),
    // the ruler TOP.
    let m = Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE);
    let rect = Rect::from_min_max(pos2(0.0, 0.0), pos2(800.0, 448.0));
    let l = Layout::new(rect, 4, 1, &m, 192.0, RowView::default());
    let r = |x0, y0, x1, y1| Rect::from_min_max(pos2(x0, y0), pos2(x1, y1));
    let z = step_seq::zones(&l);
    // Touch scroll y: the full width below the ruler; touch scroll x: the grid columns from the ruler's top.
    assert_eq!(
        z.touch,
        [r(290.0, 0.0, 800.0, 448.0), r(0.0, TOP, 800.0, 448.0)]
    );
    // Wheel zoom x over the ruler band, zoom y over the header column; the grid scrolls and takes the pinch.
    assert_eq!(
        z.wheel_zoom,
        [r(290.0, 0.0, 800.0, TOP), r(0.0, TOP, 288.0, 448.0)]
    );
    assert_eq!(
        (z.wheel_scroll, z.pinch),
        (r(290.0, TOP, 800.0, 448.0), r(290.0, TOP, 800.0, 448.0))
    );
}

#[test]
fn one_finger_scrolls_the_rows_from_the_headers_and_both_ways_on_the_grid_and_two_zoom_both() {
    // The drum editor's zones on the scroll and zoom laws.
    use egui::TouchPhase::{End, Move, Start};
    let mut rig = Rig::new(notes_lane(&[drum(0, 0, 32.0)]));
    rig.song.channels[CH].lanes[0].clips[0].loop_len_beats = 64.0;
    let view = |rig: &Rig| *rig.view.views.values().next().unwrap();
    // Rows 12 U tall (the wheel over the headers): the 4 pads and 1.5 rows of room outgrow the rows' area.
    for _ in 0..10 {
        rig.frame(vec![
            egui::Event::PointerMoved(pos2(48.0, 100.0)),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: vec2(0.0, 300.0),
                modifiers: Modifiers::NONE,
            },
        ]);
    }
    let v = view(&rig);
    assert!((v.rows.pitch - 12.0).abs() < 1e-5, "{v:?}");
    // On the header column, 60 points up and 60 sideways: the rows follow, the time does not.
    rig.touch(Start, pos2(48.0, 300.0));
    rig.touch(Move, pos2(108.0, 240.0));
    rig.frame(vec![]);
    rig.touch(End, pos2(108.0, 240.0));
    let w = view(&rig);
    assert!(
        (w.rows.scroll - v.rows.scroll - 60.0 / 32.0).abs() < 1e-4,
        "{w:?}"
    );
    assert_eq!(w.x, v.x);
    // On empty grid: both.
    rig.touch(Start, pos2(600.0, 300.0));
    rig.touch(Move, pos2(540.0, 280.0));
    rig.frame(vec![]);
    rig.touch(End, pos2(540.0, 280.0));
    let s = view(&rig);
    assert!((s.x - w.x - 60.0 / 128.0).abs() < 1e-6, "{s:?}");
    assert!(
        (s.rows.scroll - w.rows.scroll - 20.0 / 32.0).abs() < 1e-4,
        "{s:?}"
    );
    assert!(rig.actions.is_empty(), "{:?}", rig.actions);
    // Two fingers closing 300 → 200 points sideways and 150 → 100 down: the time and the rows zoom out 1.5 times.
    rig.touch(Start, pos2(600.0, 300.0));
    rig.second(Start, pos2(900.0, 450.0));
    rig.second(Move, pos2(800.0, 400.0));
    rig.second(End, pos2(800.0, 400.0));
    rig.touch(End, pos2(600.0, 300.0));
    let p = view(&rig);
    assert!((p.k - s.k * 1.5).abs() < 1e-5, "{p:?}");
    assert!((p.rows.pitch - s.rows.pitch / 1.5).abs() < 1e-4, "{p:?}");
    assert!(rig.actions.is_empty(), "{:?}", rig.actions);
}

#[test]
fn another_clip_takes_no_fling_from_the_last_one() {
    // A fling of one clip's time does not move the next clip's view.
    let mut channel = notes_lane(&[drum(0, 0, 32.0)]);
    channel.lanes[0].add_clip(4096, 64.0);
    channel.lanes[0].clips[0].loop_len_beats = 64.0;
    let mut rig = Rig::new(channel);
    let second = |rig: &Rig| rig.view.views.get(&(CH, 0, Some(2))).map(|v| v.x);
    let from = at(3, 700.0);
    rig.frame_after(1.0 / 60.0, vec![egui::Event::PointerMoved(from)]);
    rig.frame_after(
        1.0 / 60.0,
        vec![egui::Event::PointerButton {
            pos: from,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        }],
    );
    for k in 1..=3 {
        let to = from - vec2(60.0 * k as f32, 0.0);
        rig.frame_after(1.0 / 60.0, vec![egui::Event::PointerMoved(to)]);
    }
    let to = from - vec2(180.0, 0.0);
    rig.frame_after(
        1.0 / 60.0,
        vec![egui::Event::PointerButton {
            pos: to,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }],
    );
    let view = |rig: &Rig| rig.view.views[&(CH, 0, Some(1))].x;
    let flying = view(&rig);
    rig.frame_after(1.0 / 60.0, vec![]);
    assert_ne!(view(&rig), flying, "the first clip flings");
    rig.clip = Some(2);
    for _ in 0..10 {
        rig.frame_after(1.0 / 60.0, vec![]);
    }
    assert_eq!(second(&rig), Some(0.0));
}

// The keymap's editing and zoom actions (`ui/edit_keys.rs`).

impl Rig {
}

/// The play box fades as the drum editor's (`timeline::knob_shown`, through `timeline::PlayheadKnob`): out while
/// playing, so that only the line shows, and back in once stopped.
#[test]
fn the_play_box_fades_out_while_playing_and_back_in_stopped() {
    let menu = theme_by_id(0).colors.menu_fill.to_color32();
    let mut rig = Rig::new(notes_lane(&[drum(0, 0, 32.0)]));
    let disc = |rig: &Rig| {
        crate::ui::timeline::testing::play_boxes(&rig.shapes)
            .next()
            .map(|b| b.fill)
    };
    rig.view.playhead = 0;
    for (playing, fill) in [(false, Some(menu)), (true, None), (false, Some(menu))] {
        rig.view.playing = playing;
        for _ in 0..40 {
            rig.frame(vec![]);
        }
        assert_eq!(disc(&rig), fill, "playing {playing}");
    }
}
