use std::time::Duration;

use adam::Time;

#[test]
fn accumulator_yields_whole_steps_only() {
    let mut time = Time::with_fixed_dt(Duration::from_millis(10));
    time.tick();

    // Nothing accumulated yet.
    assert!(time.consume_fixed_step().is_none());

    // Feed exactly two steps.
    time.tick();
    time.set_scale(0.0);
    assert_eq!(time.pending_steps(), 0);
}

#[test]
fn a_long_frame_is_clamped() {
    let mut time = Time::with_fixed_dt(Duration::from_millis(10));
    std::thread::sleep(Duration::from_millis(30));
    let delta = time.tick();

    // Clamped to 250ms max, so a stall cannot queue an unbounded step burst.
    assert!(delta <= Duration::from_millis(250));
}

#[test]
fn pending_steps_counts_the_backlog() {
    let mut time = Time::with_fixed_dt(Duration::from_millis(1));
    std::thread::sleep(Duration::from_millis(20));
    time.tick();

    assert!(time.pending_steps() >= 10, "got {}", time.pending_steps());
}

#[test]
fn scale_zero_freezes_simulation_but_not_the_clock() {
    let mut time = Time::with_fixed_dt(Duration::from_millis(1));
    time.set_scale(0.0);

    std::thread::sleep(Duration::from_millis(5));
    time.tick();

    assert_eq!(time.pending_steps(), 0, "scaled time still accumulated");
    assert!(time.elapsed() > Duration::ZERO, "real clock stopped");
}

#[test]
fn consuming_drains_the_accumulator() {
    let mut time = Time::with_fixed_dt(Duration::from_micros(100));
    time.tick();
    time.set_scale(0.0);

    // Push in enough real time for many steps.
    std::thread::sleep(Duration::from_millis(5));
    time.tick();

    let total = time.pending_steps();
    let mut drained = 0;
    while time.consume_fixed_step().is_some() {
        drained += 1;
        assert!(drained < 100_000, "consume_fixed_step never converges");
    }

    assert_eq!(drained, total);
    assert_eq!(time.pending_steps(), 0);
}

#[test]
fn frame_counter_increments() {
    let mut time = Time::new();
    assert_eq!(time.frame(), 0);
    for _ in 0..5 {
        time.tick();
    }
    assert_eq!(time.frame(), 5);
}

#[test]
fn zero_fixed_step_is_rejected() {
    // A zero step would make `consume_fixed_step` loop forever and make
    // `pending_steps` divide by zero.
    let time = Time::with_fixed_dt(Duration::ZERO);
    assert!(time.fixed_dt() >= Duration::from_micros(1));
}
