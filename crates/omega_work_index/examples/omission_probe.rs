fn main() {
    let gate = omega_work_index::DogfoodFixtureGate::from_process_environment();
    let loaded = omega_work_index::DogfoodFixtureAdapter::load(gate);
    println!(
        "debug_assertions={} enabled={} loaded_some={:?}",
        cfg!(debug_assertions),
        gate.enabled(),
        loaded.map(|o| o.is_some())
    );
}
