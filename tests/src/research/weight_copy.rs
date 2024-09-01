use crate::mock::*;
use pallet_offworker::ConsensusSimulationResult;
use pallet_subnet_emission::{
    subnet_consensus::yuma::{YumaEpoch, YumaParams},
    SubnetConsensusType,
};
use pallet_subnet_emission_api::SubnetConsensus;
use pallet_subspace::{
    MaxAllowedWeights, MaxRegistrationsPerBlock, MaxWeightAge, MinAllowedWeights, MinWeightStake,
    Tempo, N,
};
use serde_json::Value;
use sp_runtime::Percent;
use std::{fs::File, io::Read, path::PathBuf};

// TODO:
// ## BUILDUP
// Build subnet modules according to the data

// ## Get random 10 epoch information
// - Download weight information
// - Download registration information

/// Super simple test ensuring it is not profitable to copy weights

#[test]
fn test_backtest_simulation() {
    new_test_ext().execute_with(|| {
        const NETUID: u16 = 0;
        const TEMPO: u64 = 100;
        const UNIVERSAL_PENDING_EMISSION: u64 = to_nano(100);
        const DELEGATION_FEE: Percent = Percent::from_percent(5);
        const JSON_NETUID: &str = "8";

        // Setup the subnet
        register_subnet(u32::MAX, 0).unwrap();
        SubnetConsensusType::<Test>::set(NETUID, Some(SubnetConsensus::Yuma));
        MaxWeightAge::<Test>::insert(NETUID, 100_000);
        MinWeightStake::<Test>::set(0);
        Tempo::<Test>::insert(NETUID, TEMPO as u16);

        // Setup the network
        zero_min_burn();
        MaxRegistrationsPerBlock::<Test>::set(1000);
        MinAllowedWeights::<Test>::insert(NETUID, 1);

        dbg!(MaxAllowedWeights::<Test>::get(NETUID));
        let json = load_json_data();
        register_modules_from_json(&json, NETUID);
        step_block(1);

        let copier_uid: u16 = N::<Test>::get(NETUID);

        let first_block = json["weights"].as_object().unwrap().keys().next().unwrap();
        set_weights_for_block(
            &json["weights"][first_block][JSON_NETUID],
            NETUID,
            copier_uid,
        );

        let last_params = YumaParams::<Test>::new(NETUID, UNIVERSAL_PENDING_EMISSION).unwrap();
        let last_output = YumaEpoch::<Test>::new(NETUID, last_params).run().unwrap();

        let consensus = last_output.consensus;

        add_weight_copier(
            NETUID,
            copier_uid as u32,
            (0..consensus.len() as u16).collect(),
            consensus,
        );

        let mut simulation_result = ConsensusSimulationResult::default();

        for (block_number, block_weights) in json["weights"].as_object().unwrap() {
            if block_number == first_block {
                continue;
            }

            set_weights_for_block(&block_weights[JSON_NETUID], NETUID, copier_uid);

            let last_params = YumaParams::<Test>::new(NETUID, UNIVERSAL_PENDING_EMISSION).unwrap();
            let last_output = YumaEpoch::<Test>::new(NETUID, last_params).run().unwrap();
            simulation_result.update(last_output, TEMPO, copier_uid, DELEGATION_FEE);

            if pallet_offworker::is_copying_irrational(simulation_result.clone()) {
                println!("Copying became irrational at block {}", block_number);
                break;
            }

            step_block(1);
        }

        dbg!(&simulation_result);
        assert!(
            pallet_offworker::is_copying_irrational(simulation_result),
            "Copying should have become irrational"
        );
    });
}

fn load_json_data() -> Value {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("../snapshots/weights_stake.json");
    let mut file = File::open(path).expect("Failed to open weights_stake.json");
    let mut contents = String::new();
    file.read_to_string(&mut contents).expect("Failed to read file");
    serde_json::from_str(&contents).expect("Failed to parse JSON")
}

fn register_modules_from_json(json: &Value, netuid: u16) {
    if let Some(stake_map) = json["stake"].as_object() {
        let mut sorted_uids: Vec<u16> =
            stake_map.keys().filter_map(|uid_str| uid_str.parse::<u16>().ok()).collect();
        sorted_uids.sort_unstable();

        for uid in sorted_uids {
            if let Some(stake_value) = stake_map.get(&uid.to_string()) {
                let stake: u64 = stake_value.as_u64().expect("Failed to parse stake value");
                register_module(netuid, uid as u32, stake, false).unwrap();
            }
        }
    }
}

fn set_weights_for_block(block_weights: &Value, netuid: u16, copier_uid: u16) {
    let mut sorted_uids: Vec<u16> = block_weights
        .as_object()
        .unwrap()
        .keys()
        .filter_map(|uid_str| uid_str.parse::<u16>().ok())
        .collect();
    sorted_uids.sort_unstable();

    for uid in sorted_uids {
        if uid == copier_uid {
            continue;
        }

        let stake = SubspaceMod::get_delegated_stake(&(uid as u32));
        if stake == 0 {
            continue;
        }

        if let Some(weights) = block_weights[&uid.to_string()].as_array() {
            let weight_vec: Vec<(u16, u16)> = weights
                .iter()
                .filter_map(|w| {
                    let pair = w.as_array()?;
                    Some((pair[0].as_u64()? as u16, pair[1].as_u64()? as u16))
                })
                .collect();

            if weight_vec.len() <= 1 {
                continue;
            }

            let filtered_weight_vec: Vec<(u16, u16)> =
                weight_vec.into_iter().filter(|(weight_uid, _)| *weight_uid != uid).collect();

            let (uids, values): (Vec<u16>, Vec<u16>) = filtered_weight_vec.into_iter().unzip();

            debug_assert!(!uids.contains(&uid), "UIDs should not contain the same UID");

            set_weights(netuid, uid as u32, uids, values);
        }
    }
}

// ? Experimentation
use rand::{thread_rng, Rng};

fn display_weight_deltas(weights: &[u16]) {
    let total_weight: u32 = weights.iter().map(|&w| w as u32).sum();
    let percentages: Vec<f64> =
        weights.iter().map(|&w| (w as f64 / total_weight as f64) * 100.0).collect();

    println!("Total weight: {}", total_weight);
    println!("Weight percentages:");
    for (i, percentage) in percentages.iter().enumerate() {
        println!("Weight {}: {:.2}%", i, percentage);
    }

    let max_delta = percentages.iter().max_by(|a, b| a.partial_cmp(b).unwrap()).unwrap()
        - percentages.iter().min_by(|a, b| a.partial_cmp(b).unwrap()).unwrap();
    println!("Max delta between weights: {:.2}%", max_delta);
}

#[test]
fn test_weight_copying_irrationality() {
    new_test_ext().execute_with(|| {
        // Parameters
        const NETUID: u16 = 0;
        const NUM_MODULES: u16 = 10;
        const WEIGHT_VECTOR_LENGTH: usize = 4;
        const COPIER_UID: u16 = 10;
        const MAX_ITERATIONS: usize = 100;
        const MODULE_STAKE: u64 = to_nano(10_000);
        const UNIVERSAL_PENDING_EMISSION: u64 = to_nano(100);
        const DELEGATION_FEE: Percent = Percent::from_percent(5);
        const TEMPO: u64 = 100;

        // Setup the subnet
        register_subnet(u32::MAX, 0).unwrap();
        SubnetConsensusType::<Test>::set(NETUID, Some(SubnetConsensus::Yuma));
        MaxWeightAge::<Test>::insert(NETUID, 100_000);
        Tempo::<Test>::insert(NETUID, TEMPO as u16);

        // Setup the network
        zero_min_burn();
        MaxRegistrationsPerBlock::<Test>::set(1000);

        // Register 10 modules
        register_n_modules(NETUID, NUM_MODULES, MODULE_STAKE, false);

        step_block(1);
        let mut consensus_weights = vec![1000u16; WEIGHT_VECTOR_LENGTH];
        let uid_vector: Vec<u16> = (0..WEIGHT_VECTOR_LENGTH as u16).collect();

        // Assert different vector size
        assert_eq!(consensus_weights.len(), uid_vector.len());

        // Set initial weights for miners
        for key in 0..NUM_MODULES as u16 {
            if !uid_vector.contains(&key) {
                set_weights(
                    NETUID,
                    key as u32,
                    uid_vector.clone(),
                    consensus_weights.clone(),
                );
            }
        }

        // Add weight copier that the simulated consensus will use
        add_weight_copier(
            NETUID,
            COPIER_UID as u32,
            uid_vector.clone(),
            consensus_weights.clone(),
        );

        // Initialize the value of default consensus simulation struct
        let mut simulation_result: ConsensusSimulationResult<Test> =
            ConsensusSimulationResult::default();

        for iteration in 0..MAX_ITERATIONS {
            // Create parameters out of setup that was made before the loop
            // After first iteration, this will be updated due to dynamically changing weights
            let last_params = YumaParams::<Test>::new(NETUID, UNIVERSAL_PENDING_EMISSION).unwrap();

            let last_output = YumaEpoch::<Test>::new(NETUID, last_params).run().unwrap();

            simulation_result.update(last_output, TEMPO, COPIER_UID, DELEGATION_FEE);

            let mut rng = thread_rng();
            let change_probability = 0.7; // Increased from 0.5

            for weight in consensus_weights.iter_mut() {
                if rng.gen_bool(change_probability) {
                    if rng.gen_bool(0.2) {
                        // 20% chance of extreme change
                        *weight = rng.gen_range(1..=65535); // Completely random new weight
                    } else {
                        let change_factor = 1.0 + rng.gen_range(-0.2..0.2); // -20% to +20% change
                        *weight = (*weight as f32 * change_factor).round() as u16;
                    }
                }
            }
            display_weight_deltas(&consensus_weights);

            for key in 0..NUM_MODULES as u16 {
                if !uid_vector.contains(&key) && key != COPIER_UID {
                    set_weights(
                        NETUID,
                        key as u32,
                        uid_vector.clone(),
                        consensus_weights.clone(),
                    );
                }
            }
            // Check if copying is irrational, if so, break the loop
            if pallet_offworker::is_copying_irrational(simulation_result.clone()) {
                println!(
                    "Copying became irrational after {} iterations",
                    iteration + 1
                );
                break;
            }
        }

        dbg!(&simulation_result);
        // Final check outside the loop
        let is_copy_ira = pallet_offworker::is_copying_irrational(simulation_result.clone());
        assert!(is_copy_ira, "Copying should have become irrational");
    });
}
