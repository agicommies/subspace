use crate::mock::*;
use core::panic;
use frame_support::IterableStorageMap;
use pallet_offworker::ConsensusSimulationResult;
use pallet_subnet_emission::{
    subnet_consensus::yuma::{YumaEpoch, YumaParams},
    SubnetConsensusType,
};
use pallet_subnet_emission_api::SubnetConsensus;
use pallet_subspace::{
    BondsMovingAverage, Dividends, LastUpdate, MaxAllowedModules, MaxAllowedUids,
    MaxAllowedValidators, MaxAllowedWeights, MaxEncryptionPeriod, MaxRegistrationsPerBlock,
    MaxWeightAge, MinAllowedWeights, MinWeightStake, RegistrationBlock, Tempo, TotalStake,
    ValidatorPermits, Weights, N,
};
use serde_json::Value;
use sp_runtime::Percent;
use std::{any::Any, fs::File, io::Read, path::PathBuf};

// TODO:
// ## BUILDUP
// Build subnet modules according to the data

// ## Get random 10 epoch information
// - Download weight information
// - Download registration information

/// Super simple test ensuring it is not profitable to copy weights

// TODO:
// thing bonds

#[test]
fn test_backtest_simulation() {
    new_test_ext().execute_with(|| {
        const NETUID: u16 = 0;
        const TEMPO: u64 = 100;
        const UNIVERSAL_PENDING_EMISSION: u64 = to_nano(100);
        const DELEGATION_FEE: Percent = Percent::from_percent(5);
        const JSON_NETUID: &str = "1"; // ! dont forget to change
        const MAX_EPOCHS: usize = 50;

        setup_subnet(NETUID, TEMPO);

        let json = load_json_data();
        let first_block =
            json["weights"].as_object().unwrap().keys().next().unwrap().parse().unwrap();

        // Register at genesis
        register_modules_from_json(&json, NETUID);

        // Set block number the simulation will start from
        System::set_block_number(first_block);
        // Overwrite last update and registration blocks
        make_parameter_consensus_overwrites(NETUID, first_block, &json, None);

        let copier_uid = setup_copier(
            NETUID,
            &json,
            first_block,
            JSON_NETUID,
            UNIVERSAL_PENDING_EMISSION,
        );

        let (simulation_result, iter_cnt) = run_simulation(
            NETUID,
            copier_uid,
            &json,
            first_block,
            JSON_NETUID,
            TEMPO,
            DELEGATION_FEE,
            MAX_EPOCHS,
            UNIVERSAL_PENDING_EMISSION,
        );

        dbg!(&simulation_result);
        assert!(
            pallet_offworker::is_copying_irrational(simulation_result.clone())
                || iter_cnt >= MAX_EPOCHS,
            "Copying should have become irrational or max iterations should have been reached"
        );
    });
}

fn setup_subnet(netuid: u16, tempo: u64) {
    register_subnet(u32::MAX, 0).unwrap();
    SubnetConsensusType::<Test>::set(netuid, Some(SubnetConsensus::Yuma));
    MaxWeightAge::<Test>::insert(netuid, 999_999);
    MinWeightStake::<Test>::set(0);
    Tempo::<Test>::insert(netuid, tempo as u16);
    MaxEncryptionPeriod::<Test>::insert(netuid, 999_999);
    BondsMovingAverage::<Test>::insert(netuid, 0);
    zero_min_burn();
    MaxRegistrationsPerBlock::<Test>::set(2_000);
    MinAllowedWeights::<Test>::insert(netuid, 1);
    MaxAllowedUids::<Test>::set(netuid, 10_000);
    MaxAllowedWeights::<Test>::set(netuid, 20_000);
    MaxAllowedValidators::<Test>::set(netuid, Some(0_000));
}

fn setup_copier(
    netuid: u16,
    json: &Value,
    first_block: u64,
    json_netuid: &str,
    universal_pending_emission: u64,
) -> u16 {
    // Copier will be appended
    let copier_uid: u16 = N::<Test>::get(netuid);

    // All registered modules will get their weights inserted
    // ! This does not update the last update
    insert_weights_for_block(
        &json["weights"][&first_block.to_string()][json_netuid],
        netuid,
        copier_uid,
    );

    let last_params = YumaParams::<Test>::new(netuid, universal_pending_emission).unwrap();
    let last_output = YumaEpoch::<Test>::new(netuid, last_params).run().unwrap();
    last_output.clone().apply();

    // dbg!(Dividends::<Test>::iter()
    //     .map(|(_, d)| d.iter().map(|&x| x as u64).sum::<u64>())
    //     .sum::<u64>());

    let consensus = last_output.consensus;

    add_weight_copier(
        netuid,
        copier_uid as u32,
        (0..consensus.len() as u16).collect(),
        consensus,
    );

    copier_uid
}

fn run_simulation(
    netuid: u16,
    copier_uid: u16,
    json: &Value,
    first_block: u64,
    json_netuid: &str,
    tempo: u64,
    delegation_fee: Percent,
    max_epochs: usize,
    universal_pending_emission: u64,
) -> (ConsensusSimulationResult<Test>, usize) {
    let mut simulation_result = ConsensusSimulationResult::default();
    let mut iter_counter = 0;

    let copier_last_update = SubspaceMod::get_last_update_for_uid(netuid, copier_uid);

    for (block_number, block_weights) in json["weights"].as_object().unwrap() {
        let block_number: u64 = block_number.parse().unwrap();
        if block_number == first_block {
            continue;
        }
        dbg!(iter_counter);

        System::set_block_number(block_number);

        make_parameter_consensus_overwrites(netuid, block_number, &json, Some(copier_last_update));

        let weights: &Value = &block_weights[json_netuid];

        insert_weights_for_block(weights, netuid, copier_uid);

        let last_params = YumaParams::<Test>::new(netuid, universal_pending_emission).unwrap();
        let last_output = YumaEpoch::<Test>::new(netuid, last_params).run().unwrap();
        last_output.clone().apply();

        // Dividends sum, total stake
        // dbg!(Dividends::<Test>::iter()
        //     .map(|(_, d)| d.iter().map(|&x| x as u64).sum::<u64>())
        //     .sum::<u64>());
        // dbg!(TotalStake::<Test>::get());

        dbg!("applied");
        simulation_result.update(last_output, tempo, copier_uid, delegation_fee);

        if pallet_offworker::is_copying_irrational(simulation_result.clone()) {
            println!("Copying became irrational at block {}", block_number);
            break;
        }

        step_block(1);
        iter_counter += 1;

        if iter_counter >= max_epochs {
            println!(
                "Max iterations ({}) reached without copying becoming irrational",
                max_epochs
            );
            break;
        }
    }

    (simulation_result, iter_counter)
}
fn load_json_data() -> Value {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("../snapshots/sn1_weights_stake.json");
    let mut file = File::open(path).expect("Failed to open weights_stake.json");
    let mut contents = String::new();
    file.read_to_string(&mut contents).expect("Failed to read file");
    serde_json::from_str(&contents).expect("Failed to parse JSON")
}

// see if sorting might not be problmeatic
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

fn insert_weights_for_block(block_weights: &Value, netuid: u16, copier_uid: u16) {
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

            // Display weight delta before setting weights
            // display_weight_deltas(&values);

            debug_assert!(!uids.contains(&uid), "UIDs should not contain the same UID");

            // // Give everyone a validator permit
            let n = N::<Test>::get(netuid);
            let permits = vec![true; n as usize + 1];
            ValidatorPermits::<Test>::insert(netuid, permits);

            // This avoids setting last update
            // dbg!(&values);
            // dbg!(&normalized_values);
            let zipped_weights: Vec<(u16, u16)> = uids.iter().copied().zip(values).collect();
            Weights::<Test>::insert(netuid, uid, zipped_weights);
        }
    }
}

fn get_value_for_block(module: &str, block_number: u64, json: &Value) -> Vec<u64> {
    let stuff = json[module].as_object().unwrap();
    let stuff_vec: Vec<u64> = stuff[&block_number.to_string()]
        .as_object()
        .unwrap()
        .values()
        .filter_map(|v| v.as_u64())
        .collect();
    stuff_vec
}

fn make_parameter_consensus_overwrites(
    netuid: u16,
    block: u64,
    json: &Value,
    copier_last_update: Option<u64>,
) {
    // Overwrite the last update completelly to the mainnet state
    let mut last_update_vec = get_value_for_block("last_update", block, &json);
    if let Some(copier_last_update) = copier_last_update {
        last_update_vec.push(copier_last_update);
    }

    LastUpdate::<Test>::insert(netuid, last_update_vec);
    // Overwrite the registration block
    let registration_blocks_vec = get_value_for_block("registration_blocks", block, &json);
    for (i, block) in registration_blocks_vec.iter().enumerate() {
        RegistrationBlock::<Test>::insert(netuid, i as u16, *block);
    }

    // dbg!(RegistrationBlock::<Test>::iter().collect::<Vec<_>>());
    // dbg!(LastUpdate::<Test>::iter().collect::<Vec<_>>());
}

// // ? Experimentation
// use rand::{thread_rng, Rng};

// fn display_weight_deltas(weights: &[u16]) {
//     let total_weight: u32 = weights.iter().map(|&w| w as u32).sum();
//     let percentages: Vec<f64> =
//         weights.iter().map(|&w| (w as f64 / total_weight as f64) * 100.0).collect();

//     let max_delta = percentages.iter().max_by(|a, b| a.partial_cmp(b).unwrap()).unwrap()
//         - percentages.iter().min_by(|a, b| a.partial_cmp(b).unwrap()).unwrap();
//     println!("Max delta between weights: {:.2}%", max_delta);
//     // dbg!(weights.len());
// }

// #[test]
// fn test_weight_copying_irrationality() {
//     new_test_ext().execute_with(|| {
//         // Parameters
//         const NETUID: u16 = 0;
//         const NUM_MODULES: u16 = 10;
//         const WEIGHT_VECTOR_LENGTH: usize = 4;
//         const COPIER_UID: u16 = 10;
//         const MAX_ITERATIONS: usize = 100;
//         const MODULE_STAKE: u64 = to_nano(10_000);
//         const UNIVERSAL_PENDING_EMISSION: u64 = to_nano(100);
//         const DELEGATION_FEE: Percent = Percent::from_percent(5);
//         const TEMPO: u64 = 100;

//         // Setup the subnet
//         register_subnet(u32::MAX, 0).unwrap();
//         SubnetConsensusType::<Test>::set(NETUID, Some(SubnetConsensus::Yuma));
//         MaxWeightAge::<Test>::insert(NETUID, 100_000);
//         Tempo::<Test>::insert(NETUID, TEMPO as u16);

//         // Setup the network
//         zero_min_burn();
//         MaxRegistrationsPerBlock::<Test>::set(1000);

//         // Register 10 modules
//         register_n_modules(NETUID, NUM_MODULES, MODULE_STAKE, false);

//         step_block(1);
//         let mut consensus_weights = vec![1000u16; WEIGHT_VECTOR_LENGTH];
//         let uid_vector: Vec<u16> = (0..WEIGHT_VECTOR_LENGTH as u16).collect();

//         // Assert different vector size
//         assert_eq!(consensus_weights.len(), uid_vector.len());

//         // Set initial weights for miners
//         for key in 0..NUM_MODULES as u16 {
//             if !uid_vector.contains(&key) {
//                 set_weights(
//                     NETUID,
//                     key as u32,
//                     uid_vector.clone(),
//                     consensus_weights.clone(),
//                 );
//             }
//         }

//         // Add weight copier that the simulated consensus will use
//         add_weight_copier(
//             NETUID,
//             COPIER_UID as u32,
//             uid_vector.clone(),
//             consensus_weights.clone(),
//         );

//         // Initialize the value of default consensus simulation struct
//         let mut simulation_result: ConsensusSimulationResult<Test> =
//             ConsensusSimulationResult::default();

//         for iteration in 0..MAX_ITERATIONS {
//             // Create parameters out of setup that was made before the loop
//             // After first iteration, this will be updated due to dynamically changing weights
//             let last_params = YumaParams::<Test>::new(NETUID,
// UNIVERSAL_PENDING_EMISSION).unwrap();

//             let last_output = YumaEpoch::<Test>::new(NETUID, last_params).run().unwrap();

//             simulation_result.update(last_output, TEMPO, COPIER_UID, DELEGATION_FEE);

//             let mut rng = thread_rng();
//             let change_probability = 0.7; // Increased from 0.5

//             for weight in consensus_weights.iter_mut() {
//                 if rng.gen_bool(change_probability) {
//                     if rng.gen_bool(0.2) {
//                         // 20% chance of extreme change
//                         *weight = rng.gen_range(1..=65535); // Completely random new weight
//                     } else {
//                         let change_factor = 1.0 + rng.gen_range(-0.2..0.2); // -20% to +20%
// change                         *weight = (*weight as f32 * change_factor).round() as u16;
//                     }
//                 }
//             }
//             display_weight_deltas(&consensus_weights);

//             for key in 0..NUM_MODULES as u16 {
//                 if !uid_vector.contains(&key) && key != COPIER_UID {
//                     set_weights(
//                         NETUID,
//                         key as u32,
//                         uid_vector.clone(),
//                         consensus_weights.clone(),
//                     );
//                 }
//             }
//             // Check if copying is irrational, if so, break the loop
//             if pallet_offworker::is_copying_irrational(simulation_result.clone()) {
//                 println!(
//                     "Copying became irrational after {} iterations",
//                     iteration + 1
//                 );
//                 break;
//             }
//         }

//         dbg!(&simulation_result);
//         // Final check outside the loop
//         let is_copy_ira = pallet_offworker::is_copying_irrational(simulation_result.clone());
//         assert!(is_copy_ira, "Copying should have become irrational");
//     });
// }
