//! Scenario testkit: deterministic, headless, machine-readable.
//!
//! A [`Scenario`] owns its world, runs a fixed number of ticks, then
//! verifies checks. The runner produces a serializable [`ScenarioReport`]
//! so results can be asserted in tests, diffed in CI, and inspected by
//! humans and agents alike. Input/event replay (M4 roadmap) will build on
//! `aether_core::events::TimestampedLog`, which provides the recording
//! primitives today.

use aether_core::rng::SeededRng;
use aether_core::tick::Tick;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Context handed to scenarios: deterministic time, rng, metric recording,
/// and check registration.
pub struct ScenarioCtx {
    pub tick: Tick,
    pub dt: f64,
    pub rng: SeededRng,
    checks: Vec<CheckResult>,
    metrics: BTreeMap<String, Vec<f64>>,
}

impl ScenarioCtx {
    fn new(dt: f64, seed: u64) -> Self {
        Self {
            tick: Tick(0),
            dt,
            rng: SeededRng::new(seed),
            checks: Vec::new(),
            metrics: BTreeMap::new(),
        }
    }

    /// Record a named boolean check with context.
    pub fn check(&mut self, name: &str, passed: bool, detail: impl Into<String>) {
        self.checks.push(CheckResult {
            name: name.to_string(),
            passed,
            detail: detail.into(),
        });
    }

    /// Check that two scalars agree within `epsilon`.
    pub fn check_near(&mut self, name: &str, actual: f64, expected: f64, epsilon: f64) {
        let passed = (actual - expected).abs() <= epsilon;
        self.check(
            name,
            passed,
            format!("actual={actual:.6} expected={expected:.6} eps={epsilon}"),
        );
    }

    /// Append a sample to a named metric series.
    pub fn record(&mut self, metric: &str, value: f64) {
        self.metrics
            .entry(metric.to_string())
            .or_default()
            .push(value);
    }
}

/// A deterministic, headless test scenario.
pub trait Scenario {
    fn name(&self) -> &'static str;

    /// Fixed tick rate for this scenario.
    fn tick_rate(&self) -> f64 {
        30.0
    }

    /// How many ticks to simulate.
    fn ticks(&self) -> u64;

    /// RNG seed (override for seed-sweep testing).
    fn seed(&self) -> u64 {
        0xA37E_0001
    }

    fn setup(&mut self, ctx: &mut ScenarioCtx);

    fn tick(&mut self, ctx: &mut ScenarioCtx);

    /// Called after the last tick; register final checks here.
    fn verify(&mut self, ctx: &mut ScenarioCtx);
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckResult {
    pub name: String,
    pub passed: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricSummary {
    pub count: usize,
    pub min: f64,
    pub max: f64,
    pub mean: f64,
    pub last: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioReport {
    pub scenario: String,
    pub ticks_run: u64,
    pub tick_rate: f64,
    pub seed: u64,
    pub passed: bool,
    pub checks: Vec<CheckResult>,
    pub metrics: BTreeMap<String, MetricSummary>,
}

impl ScenarioReport {
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("report serializes")
    }

    pub fn failed_checks(&self) -> Vec<&CheckResult> {
        self.checks.iter().filter(|c| !c.passed).collect()
    }
}

fn summarize(samples: &[f64]) -> MetricSummary {
    let count = samples.len();
    if count == 0 {
        return MetricSummary {
            count: 0,
            min: 0.0,
            max: 0.0,
            mean: 0.0,
            last: 0.0,
        };
    }
    let mut min = f64::MAX;
    let mut max = f64::MIN;
    let mut sum = 0.0;
    for &s in samples {
        min = min.min(s);
        max = max.max(s);
        sum += s;
    }
    MetricSummary {
        count,
        min,
        max,
        mean: sum / count as f64,
        last: samples[count - 1],
    }
}

/// Run a scenario to completion and produce its report.
pub fn run_scenario(scenario: &mut dyn Scenario) -> ScenarioReport {
    let dt = 1.0 / scenario.tick_rate();
    let mut ctx = ScenarioCtx::new(dt, scenario.seed());

    scenario.setup(&mut ctx);
    let total = scenario.ticks();
    for _ in 0..total {
        scenario.tick(&mut ctx);
        ctx.tick = ctx.tick.next();
    }
    scenario.verify(&mut ctx);

    let passed = ctx.checks.iter().all(|c| c.passed) && !ctx.checks.is_empty();
    ScenarioReport {
        scenario: scenario.name().to_string(),
        ticks_run: total,
        tick_rate: scenario.tick_rate(),
        seed: scenario.seed(),
        passed,
        checks: ctx.checks,
        metrics: ctx
            .metrics
            .iter()
            .map(|(k, v)| (k.clone(), summarize(v)))
            .collect(),
    }
}

/// Run and panic with the JSON report on failure — for use inside `#[test]`s.
pub fn assert_scenario(scenario: &mut dyn Scenario) -> ScenarioReport {
    let report = run_scenario(scenario);
    if !report.passed {
        panic!(
            "scenario '{}' failed:\n{}",
            report.scenario,
            report.to_json()
        );
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Constant-acceleration kinematics as a self-test of the runner.
    struct Kinematics {
        position: f64,
        velocity: f64,
    }

    impl Scenario for Kinematics {
        fn name(&self) -> &'static str {
            "kinematics-selftest"
        }

        fn ticks(&self) -> u64 {
            300
        }

        fn setup(&mut self, _ctx: &mut ScenarioCtx) {
            self.position = 0.0;
            self.velocity = 0.0;
        }

        fn tick(&mut self, ctx: &mut ScenarioCtx) {
            const ACCEL: f64 = 2.0;
            self.velocity += ACCEL * ctx.dt;
            self.position += self.velocity * ctx.dt;
            ctx.record("velocity", self.velocity);
        }

        fn verify(&mut self, ctx: &mut ScenarioCtx) {
            // 10 seconds at 2 m/s^2: v = 20, x ~ 100 (+ discretization bias)
            ctx.check_near("final velocity", self.velocity, 20.0, 0.01);
            ctx.check_near("final position", self.position, 100.0, 0.5);
        }
    }

    #[test]
    fn runner_executes_and_passes() {
        let report = assert_scenario(&mut Kinematics {
            position: 0.0,
            velocity: 0.0,
        });
        assert_eq!(report.ticks_run, 300);
        assert!(report.metrics.contains_key("velocity"));
        assert_eq!(report.metrics["velocity"].count, 300);
    }

    struct AlwaysFails;

    impl Scenario for AlwaysFails {
        fn name(&self) -> &'static str {
            "fails"
        }
        fn ticks(&self) -> u64 {
            1
        }
        fn setup(&mut self, _ctx: &mut ScenarioCtx) {}
        fn tick(&mut self, _ctx: &mut ScenarioCtx) {}
        fn verify(&mut self, ctx: &mut ScenarioCtx) {
            ctx.check("expected failure", false, "deliberate");
        }
    }

    #[test]
    fn failing_scenario_reports_failure() {
        let report = run_scenario(&mut AlwaysFails);
        assert!(!report.passed);
        assert_eq!(report.failed_checks().len(), 1);
    }

    struct NoChecks;

    impl Scenario for NoChecks {
        fn name(&self) -> &'static str {
            "no-checks"
        }
        fn ticks(&self) -> u64 {
            1
        }
        fn setup(&mut self, _ctx: &mut ScenarioCtx) {}
        fn tick(&mut self, _ctx: &mut ScenarioCtx) {}
        fn verify(&mut self, _ctx: &mut ScenarioCtx) {}
    }

    #[test]
    fn scenario_without_checks_is_a_failure() {
        // A scenario that asserts nothing proves nothing.
        let report = run_scenario(&mut NoChecks);
        assert!(!report.passed);
    }
}
