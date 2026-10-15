use super::*;
use crate::input::SeatState;
use crate::reexports::wayland_server::protocol::wl_surface::WlSurface;
use crate::utils::SERIAL_COUNTER;

const EVENT_TIME: u32 = 10;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Target(u8);

#[derive(Debug, PartialEq, Eq)]
enum Event {
    Down,
    Up,
    Motion,
    Frame,
    Cancel,
}

#[derive(Default)]
struct State {
    seats: SeatState<Self>,
    events: Vec<(Target, Event, Serial)>,
}

impl SeatHandler for State {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = Target;

    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.seats
    }
}

impl IsAlive for Target {
    fn alive(&self) -> bool {
        true
    }
}

impl TouchTarget<State> for Target {
    fn down(&self, _: &Seat<State>, state: &mut State, _: &DownEvent, seq: Serial) {
        state.events.push((self.clone(), Event::Down, seq));
    }

    fn up(&self, _: &Seat<State>, state: &mut State, _: &UpEvent, seq: Serial) {
        state.events.push((self.clone(), Event::Up, seq));
    }

    fn motion(&self, _: &Seat<State>, state: &mut State, _: &MotionEvent, seq: Serial) {
        state.events.push((self.clone(), Event::Motion, seq));
    }

    fn frame(&self, _: &Seat<State>, state: &mut State, seq: Serial) {
        state.events.push((self.clone(), Event::Frame, seq));
    }

    fn cancel(&self, _: &Seat<State>, state: &mut State, seq: Serial) {
        state.events.push((self.clone(), Event::Cancel, seq));
    }

    fn shape(&self, _: &Seat<State>, _: &mut State, _: &ShapeEvent, _: Serial) {}

    fn orientation(&self, _: &Seat<State>, _: &mut State, _: &OrientationEvent, _: Serial) {}
}

fn setup() -> (State, TouchHandle<State>) {
    let mut state = State::default();
    let touch = state.seats.new_seat("test").add_touch();
    (state, touch)
}

fn down(state: &mut State, touch: &TouchHandle<State>, target: u8, slot: u32) {
    // Allow independent focus targets, as compositors injecting remote contacts do.
    touch.unset_grab(state);
    touch.down(
        state,
        Some((Target(target), (0.0, 0.0).into())),
        &DownEvent {
            slot: Some(slot).into(),
            location: (20.0, 30.0).into(),
            serial: SERIAL_COUNTER.next_serial(),
            time: EVENT_TIME,
        },
    );
}

fn motion(state: &mut State, touch: &TouchHandle<State>) {
    touch.motion(
        state,
        None,
        &MotionEvent {
            slot: Some(0).into(),
            location: (30.0, 40.0).into(),
            time: EVENT_TIME + 1,
        },
    );
}

fn up(state: &mut State, touch: &TouchHandle<State>) {
    touch.up(
        state,
        &UpEvent {
            slot: Some(0).into(),
            serial: SERIAL_COUNTER.next_serial(),
            time: EVENT_TIME + 2,
        },
    );
}

fn assert_canceled(state: &State, touch: &TouchHandle<State>) {
    assert_eq!(state.events.len(), 1);
    assert_eq!(state.events[0].0, Target(0));
    assert_eq!(state.events[0].1, Event::Cancel);
    assert!(touch.inner.lock().unwrap().focus.is_empty());
    assert!(!touch.is_grabbed());
}

#[test]
fn cancel_after_down_frame_notifies_target_and_clears_contact() {
    let (mut state, touch) = setup();
    down(&mut state, &touch, 0, 0);
    touch.frame(&mut state);
    state.events.clear();

    touch.cancel(&mut state);
    assert_canceled(&state, &touch);

    // No release, motion or trailing frame may leak from the canceled gesture.
    state.events.clear();
    motion(&mut state, &touch);
    up(&mut state, &touch);
    touch.frame(&mut state);
    touch.cancel(&mut state);
    assert!(state.events.is_empty());

    // Reuse the slot on another target without needing an extra touch-up first.
    down(&mut state, &touch, 1, 0);
    touch.frame(&mut state);
    assert_eq!(state.events.len(), 2);
    assert_eq!(state.events[0].0, Target(1));
    assert_eq!(state.events[0].1, Event::Down);
    assert_eq!(state.events[1].1, Event::Frame);
}

#[test]
fn cancel_after_motion_frame_notifies_target() {
    let (mut state, touch) = setup();
    down(&mut state, &touch, 0, 0);
    touch.frame(&mut state);
    motion(&mut state, &touch);
    touch.frame(&mut state);
    state.events.clear();

    touch.cancel(&mut state);
    assert_canceled(&state, &touch);
}

#[test]
fn cancel_before_frame_notifies_target() {
    let (mut state, touch) = setup();
    down(&mut state, &touch, 0, 0);
    state.events.clear();

    touch.cancel(&mut state);
    assert_canceled(&state, &touch);
}

#[test]
fn cancel_clears_pending_up_frame() {
    let (mut state, touch) = setup();
    down(&mut state, &touch, 0, 0);
    touch.frame(&mut state);
    up(&mut state, &touch);
    state.events.clear();

    touch.cancel(&mut state);
    assert_canceled(&state, &touch);
    touch.frame(&mut state);
    assert_eq!(state.events.len(), 1);
}

#[test]
fn cancel_does_not_notify_fully_released_targets() {
    let (mut state, touch) = setup();
    down(&mut state, &touch, 0, 0);
    touch.frame(&mut state);
    up(&mut state, &touch);
    touch.frame(&mut state);
    state.events.clear();

    touch.cancel(&mut state);
    assert!(state.events.is_empty());
    assert!(touch.inner.lock().unwrap().focus.is_empty());
}

#[test]
fn cancel_covers_all_targets_with_a_shared_sequence_for_client_deduplication() {
    let (mut state, touch) = setup();
    down(&mut state, &touch, 0, 0);
    down(&mut state, &touch, 0, 1);
    touch.frame(&mut state);
    down(&mut state, &touch, 1, 2);
    state.events.clear();

    touch.cancel(&mut state);
    state.events.sort_by_key(|(target, _, _)| target.0);
    assert_eq!(state.events.len(), 3);
    assert_eq!(state.events[0].0, Target(0));
    assert_eq!(state.events[1].0, Target(0));
    assert_eq!(state.events[2].0, Target(1));
    let sequence = state.events[0].2;
    assert!(state
        .events
        .iter()
        .all(|(_, event, seq)| *event == Event::Cancel && *seq == sequence));
    assert!(touch.inner.lock().unwrap().focus.is_empty());
    assert!(!touch.is_grabbed());
}
