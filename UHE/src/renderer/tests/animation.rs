//! Animation behaviour: sampling, looping, blending, state transitions and
//! compression. Pure logic only — no GPU, no window, no device.

use bifrost::animation::{
    blend_poses, compress_clip, measure_clip, sample_position, sample_rotation, Animator, Bone,
    Clip, CompressSettings, Condition, ConditionOp, Interpolation, LoopMode, Parameter, Pose,
    Skeleton, State, Transition, VectorTrack,
};
use glam::{Quat, Vec3};

fn two_bone_skeleton() -> Skeleton {
    let mut skeleton = Skeleton::new();
    let root = Bone {
        id: 0,
        local_transform: glam::Mat4::from_translation(Vec3::new(0.0, 1.0, 0.0)),
        ..Default::default()
    };
    let child = Bone {
        id: 1,
        parent_id: 0,
        local_transform: glam::Mat4::from_translation(Vec3::new(0.0, 1.0, 0.0)),
        ..Default::default()
    };
    skeleton.bones.push(root);
    skeleton.bones.push(child);
    skeleton.joint_nodes.push(1);
    skeleton.root_bone_id = 0;
    skeleton
}

fn ramp_clip(name: &str, duration: f32) -> Clip {
    let mut clip = Clip::new(name);
    clip.position_track_mut(0)
        .set_keys(&[0.0, duration], &[Vec3::ZERO, Vec3::new(0.0, 2.0, 0.0)]);
    clip.update_duration();
    clip
}

fn animator_with(clip: Clip) -> Animator {
    let mut animator = Animator::new(two_bone_skeleton());
    animator.set_clips(vec![clip]);
    animator.play_animation(0);
    animator
}

fn position_track() -> VectorTrack {
    let mut track = VectorTrack::new(0);
    track.set_keys(
        &[0.0, 1.0, 2.0],
        &[
            Vec3::ZERO,
            Vec3::new(2.0, 4.0, 0.0),
            Vec3::new(6.0, 0.0, 0.0),
        ],
    );
    track
}

#[test]
fn sampling_hits_the_exact_keys() {
    let track = position_track();
    assert_eq!(sample_position(0.0, &track), Vec3::ZERO);
    assert_eq!(sample_position(1.0, &track), Vec3::new(2.0, 4.0, 0.0));
    assert_eq!(sample_position(2.0, &track), Vec3::new(6.0, 0.0, 0.0));
}

#[test]
fn sampling_interpolates_halfway_between_keys() {
    let track = position_track();
    assert_eq!(sample_position(0.5, &track), Vec3::new(1.0, 2.0, 0.0));
    assert_eq!(sample_position(1.5, &track), Vec3::new(4.0, 2.0, 0.0));
}

#[test]
fn a_step_track_holds_the_key_before_the_time() {
    let mut track = position_track();
    track.interpolation = Interpolation::Step;
    assert_eq!(sample_position(1.999, &track), Vec3::new(2.0, 4.0, 0.0));
    assert_eq!(sample_position(2.0, &track), Vec3::new(6.0, 0.0, 0.0));
}

#[test]
fn a_looping_clip_wraps_past_its_end() {
    let mut animator = animator_with(ramp_clip("run", 1.0));
    assert_eq!(animator.loop_mode(), LoopMode::Loop);

    animator.update_animation(1.5);
    assert!((animator.current_time() - 0.5).abs() < 1e-6);
    assert!(animator.is_playing());
    assert!((animator.normalized_time() - 0.5).abs() < 1e-6);

    // A whole extra duration wraps once more.
    animator.update_animation(1.0);
    assert!((animator.current_time() - 0.5).abs() < 1e-6);
}

#[test]
fn a_non_looping_clip_clamps_at_its_end() {
    let mut animator = animator_with(ramp_clip("run", 1.0));
    animator.set_loop_mode(LoopMode::None);

    animator.update_animation(3.0);
    assert_eq!(animator.current_time(), 1.0);
    assert!(!animator.is_playing());
    assert_eq!(animator.normalized_time(), 1.0);

    // A further step does not move the play head.
    animator.update_animation(1.0);
    assert_eq!(animator.current_time(), 1.0);
    assert_eq!(
        animator.blended_pose().bones[0].translation,
        Vec3::new(0.0, 2.0, 0.0)
    );
}

#[test]
fn blend_weights_zero_half_and_one() {
    let mut from = Pose::new(1);
    from.bones[0].translation = Vec3::ZERO;
    from.bones[0].rotation = Quat::IDENTITY;
    let mut to = Pose::new(1);
    to.bones[0].translation = Vec3::new(4.0, 0.0, 0.0);
    to.bones[0].rotation = Quat::from_rotation_z(std::f32::consts::PI);
    let mut out = Pose::new(1);

    blend_poses(&from, &to, 0.0, &mut out);
    assert_eq!(out.bones[0].translation, Vec3::ZERO);
    assert_eq!(out.bones[0].rotation, Quat::IDENTITY);

    blend_poses(&from, &to, 0.5, &mut out);
    assert_eq!(out.bones[0].translation, Vec3::new(2.0, 0.0, 0.0));
    // A 180° turn is its own antipode, so the half-way rotation is a 90° turn
    // in either direction: only the magnitude is determined, not the sign.
    let quarter_turn = out.bones[0].rotation.angle_between(Quat::IDENTITY);
    assert!((quarter_turn - std::f32::consts::FRAC_PI_2).abs() < 1e-4);

    blend_poses(&from, &to, 1.0, &mut out);
    assert_eq!(out.bones[0].translation, Vec3::new(4.0, 0.0, 0.0));
    assert!(
        out.bones[0]
            .rotation
            .dot(Quat::from_rotation_z(std::f32::consts::PI))
            .abs()
            > 0.9999
    );
}

fn machine() -> bifrost::animation::AnimationStateMachine {
    let mut machine = bifrost::animation::AnimationStateMachine::new();
    machine.add_state(State {
        name: "idle".into(),
        clip_name: "Idle".into(),
        ..State::default()
    });
    machine.add_state(State {
        name: "run".into(),
        clip_name: "Run".into(),
        speed: 1.5,
        ..State::default()
    });
    machine.add_state(State {
        name: "jump".into(),
        clip_name: "Jump".into(),
        ..State::default()
    });
    machine.add_transition(Transition {
        from: "idle".into(),
        to: "run".into(),
        conditions: vec![Condition::new(
            "speed",
            ConditionOp::Greater,
            Parameter::Float(1.0),
        )],
        blend_duration: 0.2,
        ..Transition::default()
    });
    machine.add_transition(Transition {
        from: "run".into(),
        to: "jump".into(),
        conditions: vec![Condition::new(
            "jump",
            ConditionOp::Equals,
            Parameter::Bool(true),
        )],
        blend_duration: 0.1,
        ..Transition::default()
    });
    machine
}

#[test]
fn a_transition_fires_on_its_condition_and_not_before() {
    let mut machine = machine();

    // First update falls into the first registered state.
    assert_eq!(machine.update(0.0).expect("entered").state, "idle");

    machine.set_float("speed", 0.5);
    assert!(machine.update(0.0).is_none());
    assert_eq!(machine.current_state(), "idle");

    machine.set_float("speed", 1.01);
    let change = machine.update(0.0).expect("fired");
    assert_eq!(change.state, "run");
    assert_eq!(change.clip_name, "Run");
    assert_eq!(change.speed, 1.5);
    assert_eq!(change.blend_duration, 0.2);

    // The trigger condition is neither set nor a float, so nothing else fires.
    assert!(machine.update(0.0).is_none());

    machine.set_trigger("jump");
    let change = machine.update(0.0).expect("fired");
    assert_eq!(change.state, "jump");
    assert_eq!(change.blend_duration, 0.1);

    // Consumed: one trigger fires exactly once.
    assert!(machine.update(0.0).is_none());
    assert_eq!(machine.current_state(), "jump");
}

#[test]
fn the_state_machine_is_deterministic_for_a_parameter_sequence() {
    fn run() -> Vec<String> {
        let mut machine = machine();
        let mut log = Vec::new();
        for step in 0..8 {
            machine.set_float("speed", step as f32 * 0.5);
            if step == 3 {
                machine.set_trigger("jump");
            }
            let normalized = step as f32 / 8.0;
            if let Some(change) = machine.update(normalized) {
                log.push(change.state);
            }
        }
        log
    }

    let first = run();
    assert_eq!(first, run());
    assert_eq!(first.first().map(String::as_str), Some("idle"));
    assert_eq!(first.last().map(String::as_str), Some("jump"));
}

fn bowed_track_clip(samples: usize) -> Clip {
    let mut clip = Clip::new("bow");
    let times: Vec<f32> = (0..samples).map(|i| i as f32).collect();
    let values: Vec<Vec3> = (0..samples)
        .map(|i| {
            let t = i as f32 / (samples - 1) as f32;
            Vec3::new(t, 0.04 * (1.0 - (2.0 * t - 1.0).powi(2)), 0.0)
        })
        .collect();
    clip.position_track_mut(0).set_keys(&times, &values);
    clip.update_duration();
    clip
}

#[test]
fn compression_round_trip_stays_within_tolerance() {
    let settings = CompressSettings {
        position_tolerance: 0.05,
        ..CompressSettings::default()
    };
    let original = bowed_track_clip(11);
    let original_values: Vec<Vec3> = original.position_tracks[0]
        .keyframes
        .iter()
        .map(|k| k.value)
        .collect();

    let measured = measure_clip(&original, &settings);
    assert!(measured.keys_after < measured.keys_before);

    let mut compressed = original.clone();
    let stats = compress_clip(&mut compressed, &settings);
    assert_eq!(stats.keys_before, 11);
    assert_eq!(stats.keys_after, 2);
    assert!(stats.key_reduction() > 0.7);
    assert_eq!(stats.tracks_processed, 1);

    // The decoder — the ordinary sampler over the reduced track — stays inside the
    // tolerance at every time the original had a key.
    let mut worst = 0.0f32;
    for (i, value) in original_values.iter().enumerate() {
        let sampled = sample_position(i as f32, &compressed.position_tracks[0]);
        worst = worst.max((sampled - *value).length());
    }
    assert!(worst <= settings.position_tolerance, "worst {worst}");
}

#[test]
fn compression_keeps_rotations_within_tolerance() {
    let settings = CompressSettings::default();
    let mut clip = Clip::new("spin");
    let times: Vec<f32> = (0..11).map(|i| i as f32).collect();
    let values: Vec<Quat> = (0..11)
        .map(|i| Quat::from_rotation_z(i as f32 * 0.1))
        .collect();
    clip.rotation_track_mut(0).set_keys(&times, &values);
    clip.update_duration();

    let stats = compress_clip(&mut clip, &settings);
    assert!(stats.keys_after < stats.keys_before);

    let mut worst = 0.0f32;
    for (i, value) in values.iter().enumerate() {
        let sampled = sample_rotation(i as f32, &clip.rotation_tracks[0]);
        worst = worst.max(sampled.angle_between(*value));
    }
    assert!(worst <= settings.rotation_tolerance, "worst {worst}");
}

#[test]
fn compression_leaves_a_step_track_alone() {
    let mut clip = bowed_track_clip(6);
    clip.position_tracks[0].interpolation = Interpolation::Step;
    let stats = compress_clip(&mut clip, &CompressSettings::default());
    assert_eq!(stats.keys_after, stats.keys_before);
    assert_eq!(clip.position_tracks[0].keyframes.len(), 6);
}
