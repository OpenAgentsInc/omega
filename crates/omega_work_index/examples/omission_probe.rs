// The caller must run this with the development gate REQUESTED
// (`OMEGA_UI_MOCKS=1`). With the variable unset the gate is disabled in every
// profile, so `enabled=false` would prove nothing about a release build. The
// probe reports the request it observed so the proof script can refuse a run
// that was never asking for the fixture in the first place.
fn main() {
    let mocks_requested =
        std::env::var(omega_work_index::DOGFOOD_FIXTURE_ENV).as_deref() == Ok("1");
    let gate = omega_work_index::DogfoodFixtureGate::from_process_environment();
    let loaded = omega_work_index::DogfoodFixtureAdapter::load(gate);
    println!(
        "mocks_requested={} debug_assertions={} enabled={} loaded_some={:?}",
        mocks_requested,
        cfg!(debug_assertions),
        gate.enabled(),
        loaded.map(|o| o.is_some())
    );
}
