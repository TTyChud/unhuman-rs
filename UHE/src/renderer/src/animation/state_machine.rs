//! Animation state machine, ported from `Renderer3D/AnimationStateMachine.h/.cpp`.
//!
//! Conditions are data: a parameter name, a comparison operator and the value to
//! compare against, so a transition is authored, not coded. The machine owns no
//! animator and touches no scene — `update` returns the state it moved to and the
//! caller applies the pose, which is what keeps the machine usable from a test.

use std::collections::HashMap;

use crate::animation::clip::LoopMode;

/// A parameter's value. Port of `AnimationStateMachine::ParameterValue`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Parameter {
    Bool(bool),
    Int(i32),
    Float(f32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConditionOp {
    Equals,
    NotEquals,
    Greater,
    Less,
}

#[derive(Clone, Debug)]
pub struct Condition {
    pub parameter: String,
    pub op: ConditionOp,
    pub value: Parameter,
}

impl Condition {
    pub fn new(parameter: impl Into<String>, op: ConditionOp, value: Parameter) -> Self {
        Self {
            parameter: parameter.into(),
            op,
            value,
        }
    }
}

/// A transition, plus the state change the caller must apply when one fires.
#[derive(Clone, Debug)]
pub struct Transition {
    pub from: String,
    pub to: String,
    /// AND-combined.
    pub conditions: Vec<Condition>,
    pub blend_duration: f32,
    pub has_exit_time: bool,
    /// Normalised time the transition may fire at.
    pub exit_time: f32,
}

impl Default for Transition {
    fn default() -> Self {
        Self {
            from: String::new(),
            to: String::new(),
            conditions: Vec::new(),
            blend_duration: 0.25,
            has_exit_time: false,
            exit_time: 1.0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct State {
    pub name: String,
    /// Clip the animator plays while in this state.
    pub clip_name: String,
    /// Animator time scale while in this state.
    pub speed: f32,
    pub loop_mode: LoopMode,
}

impl Default for State {
    fn default() -> Self {
        Self {
            name: String::new(),
            clip_name: String::new(),
            speed: 1.0,
            loop_mode: LoopMode::Loop,
        }
    }
}

/// What the caller applies after the machine moved state.
#[derive(Clone, Debug, PartialEq)]
pub struct StateChange {
    pub state: String,
    pub clip_name: String,
    pub speed: f32,
    pub loop_mode: LoopMode,
    pub blend_duration: f32,
}

impl StateChange {
    fn from_state(state: &State, blend_duration: f32) -> Self {
        Self {
            state: state.name.clone(),
            clip_name: state.clip_name.clone(),
            speed: state.speed,
            loop_mode: state.loop_mode,
            blend_duration,
        }
    }
}

/// Port of `AnimationStateMachine`.
#[derive(Debug, Default)]
pub struct AnimationStateMachine {
    parameters: HashMap<String, Parameter>,
    triggers: HashMap<String, bool>,
    states: Vec<State>,
    transitions: Vec<Transition>,
    current_state: String,
}

impl AnimationStateMachine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_parameter(&mut self, name: &str, initial: Parameter) {
        self.parameters.insert(name.to_string(), initial);
    }

    pub fn set_bool(&mut self, name: &str, value: bool) {
        self.parameters
            .insert(name.to_string(), Parameter::Bool(value));
    }

    pub fn set_int(&mut self, name: &str, value: i32) {
        self.parameters
            .insert(name.to_string(), Parameter::Int(value));
    }

    pub fn set_float(&mut self, name: &str, value: f32) {
        self.parameters
            .insert(name.to_string(), Parameter::Float(value));
    }

    pub fn get_bool(&self, name: &str) -> bool {
        matches!(self.parameters.get(name), Some(Parameter::Bool(true)))
    }

    pub fn get_int(&self, name: &str) -> i32 {
        match self.parameters.get(name) {
            Some(Parameter::Int(v)) => *v,
            _ => 0,
        }
    }

    pub fn get_float(&self, name: &str) -> f32 {
        match self.parameters.get(name) {
            Some(Parameter::Float(v)) => *v,
            _ => 0.0,
        }
    }

    /// Triggers are one-shot booleans: consumed by the transition that used them,
    /// so a single `set_trigger` cannot re-fire a transition forever.
    pub fn set_trigger(&mut self, name: &str) {
        self.triggers.insert(name.to_string(), true);
    }

    pub fn reset_trigger(&mut self, name: &str) {
        self.triggers.insert(name.to_string(), false);
    }

    /// Adds a state, or replaces the one that already carries this name.
    pub fn add_state(&mut self, state: State) {
        if let Some(existing) = self.states.iter_mut().find(|s| s.name == state.name) {
            *existing = state;
            return;
        }
        self.states.push(state);
    }

    pub fn add_transition(&mut self, transition: Transition) {
        self.transitions.push(transition);
    }

    pub fn current_state(&self) -> &str {
        &self.current_state
    }

    /// The state the machine is in, for the per-frame speed and loop mode the
    /// caller keeps applying while it plays.
    pub fn current(&self) -> Option<&State> {
        self.states.iter().find(|s| s.name == self.current_state)
    }

    pub fn has_state(&self, name: &str) -> bool {
        self.states.iter().any(|s| s.name == name)
    }

    pub fn state(&self, name: &str) -> Option<&State> {
        self.states.iter().find(|s| s.name == name)
    }

    /// Jumps to a state immediately, with no condition or exit-time check.
    pub fn enter(&mut self, state_name: &str, blend_duration: f32) -> Option<StateChange> {
        let index = self.states.iter().position(|s| s.name == state_name)?;
        let change = StateChange::from_state(&self.states[index], blend_duration);
        self.current_state = self.states[index].name.clone();
        Some(change)
    }

    /// Applies the first transition out of the current state whose conditions hold,
    /// in registration order. `normalized_time` is the playing clip's progress, which
    /// the caller reads from the animator. Returns `None` when nothing fires.
    pub fn update(&mut self, normalized_time: f32) -> Option<StateChange> {
        if self.current_state.is_empty() {
            // No state entered yet: fall into the first registered one.
            let first = self.states.first()?;
            let change = StateChange::from_state(first, 0.0);
            self.current_state = first.name.clone();
            return Some(change);
        }

        if !self.has_state(&self.current_state) {
            return None;
        }

        let mut chosen = None;
        for (index, transition) in self.transitions.iter().enumerate() {
            if transition.from != self.current_state {
                continue;
            }
            if transition.has_exit_time && normalized_time < transition.exit_time {
                continue;
            }
            let pass = transition
                .conditions
                .iter()
                .all(|c| evaluate_condition(&self.parameters, &self.triggers, c));
            if !pass {
                continue;
            }
            chosen = Some(index);
            break;
        }

        let transition_index = chosen?;
        let state_index = self
            .states
            .iter()
            .position(|s| s.name == self.transitions[transition_index].to)?;

        let blend_duration = self.transitions[transition_index].blend_duration;
        // Consume every trigger this transition relied on, so one `set_trigger`
        // fires it exactly once.
        let consumed: Vec<String> = self.transitions[transition_index]
            .conditions
            .iter()
            .filter(|c| c.op == ConditionOp::Equals && c.value == Parameter::Bool(true))
            .map(|c| c.parameter.clone())
            .collect();
        for name in consumed {
            if let Some(trigger) = self.triggers.get_mut(&name) {
                *trigger = false;
            }
        }

        let change = StateChange::from_state(&self.states[state_index], blend_duration);
        self.current_state = self.states[state_index].name.clone();
        Some(change)
    }
}

/// Trigger conditions read from the trigger table; everything else from the
/// parameter table.
fn evaluate_condition(
    parameters: &HashMap<String, Parameter>,
    triggers: &HashMap<String, bool>,
    condition: &Condition,
) -> bool {
    if condition.op == ConditionOp::Equals && condition.value == Parameter::Bool(true) {
        if let Some(&enabled) = triggers.get(&condition.parameter) {
            return enabled;
        }
    }

    let Some(value) = parameters.get(&condition.parameter) else {
        return false;
    };
    match (value, condition.value) {
        (Parameter::Bool(a), Parameter::Bool(b)) => compare(condition.op, *a, b),
        (Parameter::Int(a), Parameter::Int(b)) => compare(condition.op, *a, b),
        (Parameter::Float(a), Parameter::Float(b)) => compare(condition.op, *a, b),
        _ => false,
    }
}

fn compare<T: PartialEq + PartialOrd>(op: ConditionOp, a: T, b: T) -> bool {
    match op {
        ConditionOp::Equals => a == b,
        ConditionOp::NotEquals => a != b,
        ConditionOp::Greater => a > b,
        ConditionOp::Less => a < b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(name: &str, clip: &str) -> State {
        State {
            name: name.to_string(),
            clip_name: clip.to_string(),
            ..State::default()
        }
    }

    fn transition(from: &str, to: &str) -> Transition {
        Transition {
            from: from.to_string(),
            to: to.to_string(),
            ..Transition::default()
        }
    }

    #[test]
    fn an_empty_machine_does_nothing() {
        let mut machine = AnimationStateMachine::new();
        assert!(machine.update(0.0).is_none());
    }

    #[test]
    fn the_first_update_enters_the_first_state() {
        let mut machine = AnimationStateMachine::new();
        machine.add_state(state("idle", "Idle"));
        let change = machine.update(0.0).expect("entered");
        assert_eq!(change.state, "idle");
        assert_eq!(change.clip_name, "Idle");
        assert_eq!(change.blend_duration, 0.0);
        assert_eq!(machine.current_state(), "idle");
    }

    #[test]
    fn a_trigger_fires_once_and_is_consumed() {
        let mut machine = AnimationStateMachine::new();
        machine.add_state(state("idle", "Idle"));
        machine.add_state(state("jump", "Jump"));
        let mut t = transition("idle", "jump");
        t.blend_duration = 0.1;
        t.conditions.push(Condition::new(
            "jump",
            ConditionOp::Equals,
            Parameter::Bool(true),
        ));
        machine.add_transition(t);

        machine.update(0.0);
        assert!(machine.update(0.0).is_none()); // no trigger set yet

        machine.set_trigger("jump");
        let change = machine.update(0.0).expect("fired");
        assert_eq!(change.state, "jump");
        assert_eq!(change.blend_duration, 0.1);
        assert!(machine.update(0.0).is_none()); // consumed
    }

    #[test]
    fn exit_time_gates_a_transition() {
        let mut machine = AnimationStateMachine::new();
        machine.add_state(state("run", "Run"));
        machine.add_state(state("idle", "Idle"));
        let mut t = transition("run", "idle");
        t.has_exit_time = true;
        t.exit_time = 0.75;
        t.conditions.push(Condition::new(
            "stop",
            ConditionOp::Equals,
            Parameter::Bool(true),
        ));
        machine.add_transition(t);

        machine.update(0.0);
        machine.set_bool("stop", true);
        assert!(machine.update(0.5).is_none());
        assert_eq!(machine.update(0.75).expect("fired").state, "idle");
    }

    #[test]
    fn float_comparisons_use_the_operator() {
        let mut machine = AnimationStateMachine::new();
        machine.add_state(state("walk", "Walk"));
        machine.add_state(state("run", "Run"));
        let mut t = transition("walk", "run");
        t.conditions.push(Condition::new(
            "speed",
            ConditionOp::Greater,
            Parameter::Float(2.0),
        ));
        machine.add_transition(t);

        machine.update(0.0);
        machine.set_float("speed", 1.5);
        assert!(machine.update(0.0).is_none());
        machine.set_float("speed", 3.0);
        assert_eq!(machine.update(0.0).expect("fired").state, "run");
    }

    #[test]
    fn an_unknown_parameter_never_matches() {
        let condition = Condition::new("flag", ConditionOp::Equals, Parameter::Bool(true));
        assert!(!evaluate_condition(
            &HashMap::new(),
            &HashMap::new(),
            &condition
        ));
    }
}
