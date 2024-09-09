use crate::mock::*;
use csv;
use pallet_offworker::ConsensusSimulationResult;
use pallet_subnet_emission::{
    subnet_consensus::yuma::{YumaEpoch, YumaParams},
    SubnetConsensusType,
};
use pallet_subnet_emission_api::SubnetConsensus;
use pallet_subspace::{
    BondsMovingAverage, Consensus, CopierMargin, LastUpdate, MaxAllowedUids, MaxAllowedWeights,
    MaxEncryptionPeriod, MaxRegistrationsPerBlock, MaxWeightAge, RegistrationBlock, Tempo,
    UseWeightsEncrytyption, ValidatorPermits, Weights, N,
};
use serde_json::Value;
use sp_runtime::Percent;
use std::{fs::File, io::Read, path::PathBuf};
use substrate_fixed::types::I64F64;

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
        // TEST SETTINGS
        const TEST_NETUID: u16 = 0;
        const TEMPO: u64 = 360;
        const UNIVERSAL_PENDING_EMISSION: u64 = to_nano(100);
        const DELEGATION_FEE: Percent = Percent::from_percent(3);
        // BACKTEST SETTINGS
        const MAX_EPOCHS: usize = 50;
        const JSON_NETUID: &str = "19"; // ! dont forget to change

        // REGISTER AND SETUP SUBNET
        setup_subnet(TEST_NETUID, TEMPO);

        let json = load_json_data();
        let first_block =
            json["weights"].as_object().unwrap().keys().next().unwrap().parse().unwrap();

        // REGISTER MODULES, AND OVERWRITE THEM TO MAINNET STATE
        // Register at genesis
        register_modules_from_json(&json, TEST_NETUID);
        // Set block number the simulation will start from
        System::set_block_number(first_block);
        // Overwrite last update and registration blocks
        make_parameter_consensus_overwrites(TEST_NETUID, first_block, &json, None);

        // Create CSV writer
        let mut wtr =
            csv::Writer::from_path("simulation_results.csv").expect("Failed to create CSV writer");
        wtr.write_record(&[
            "Copier Margin",
            "Block Number",
            "Consensus",
            "All Dividends",
            "Copier Dividend",
            "Cumulative Copier Dividends",
            "Cumulative Average Delegate Dividends",
        ])
        .expect("Failed to write CSV header");

        // Loop through different copier margin values
        for p in (0..=90).step_by(5) {
            let copier_margin = I64F64::from_num(p as f64 / 1000.0);
            CopierMargin::<Test>::insert(TEST_NETUID, copier_margin);

            // copier will set perfectly in consensus weight
            let copier_uid = setup_copier(
                TEST_NETUID,
                &json,
                first_block,
                JSON_NETUID,
                UNIVERSAL_PENDING_EMISSION,
            );

            let (simulation_result, iteration_counter) = run_simulation(
                TEST_NETUID,
                copier_uid,
                &json,
                first_block,
                JSON_NETUID,
                TEMPO,
                DELEGATION_FEE,
                MAX_EPOCHS,
                UNIVERSAL_PENDING_EMISSION,
                &mut wtr,
                copier_margin,
            );

            assert!(
                pallet_offworker::is_copying_irrational(simulation_result.clone())
                    || iteration_counter >= MAX_EPOCHS,
                "Copying should have become irrational or max iterations should have been reached"
            );

            // Reset the state for the next iteration
            System::reset_events();
            let _ = LastUpdate::<Test>::clear(u32::MAX, None);
            let _ = Weights::<Test>::clear(u32::MAX, None);
            let _ = ValidatorPermits::<Test>::clear(u32::MAX, None);
            let _ = RegistrationBlock::<Test>::clear(u32::MAX, None);
            let _ = Consensus::<Test>::clear(u32::MAX, None);
        }

        wtr.flush().expect("Failed to flush CSV writer");
    });
}

fn setup_subnet(netuid: u16, tempo: u64) {
    register_subnet(u32::MAX, 0).unwrap();
    zero_min_burn();
    SubnetConsensusType::<Test>::set(netuid, Some(SubnetConsensus::Yuma));
    Tempo::<Test>::insert(netuid, tempo as u16);
    BondsMovingAverage::<Test>::insert(netuid, 0);
    UseWeightsEncrytyption::<Test>::set(netuid, false);

    // Things that should never expire / exceed
    MaxWeightAge::<Test>::set(netuid, u64::MAX);
    MaxEncryptionPeriod::<Test>::set(netuid, u64::MAX);
    MaxRegistrationsPerBlock::<Test>::set(u16::MAX);
    MaxAllowedUids::<Test>::set(netuid, u16::MAX);
    MaxAllowedWeights::<Test>::set(netuid, u16::MAX);
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

    let v_permits = get_validator_permits(first_block, json);

    insert_weights_for_block(
        &json["weights"][&first_block.to_string()][json_netuid],
        netuid,
        v_permits,
    );

    let last_params = YumaParams::<Test>::new(netuid, universal_pending_emission).unwrap();
    let last_output = YumaEpoch::<Test>::new(netuid, last_params).run().unwrap();
    last_output.clone().apply();

    // Copier set perfectly in consensus weight
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
    wtr: &mut csv::Writer<std::fs::File>,
    copier_margin: I64F64,
) -> (ConsensusSimulationResult<Test>, usize) {
    let mut simulation_result = ConsensusSimulationResult::default();
    let mut iter_counter = 0;

    let copier_last_update = SubspaceMod::get_last_update_for_uid(netuid, copier_uid);

    for (block_number, block_weights) in json["weights"].as_object().unwrap() {
        let block_number: u64 = block_number.parse().unwrap();
        if block_number == first_block {
            continue;
        }

        System::set_block_number(block_number);
        make_parameter_consensus_overwrites(netuid, block_number, &json, Some(copier_last_update));

        let weights: &Value = &block_weights[json_netuid];

        let mut v_permits = get_validator_permits(block_number, json);
        // add permit for the copier
        v_permits.push(true);

        insert_weights_for_block(weights, netuid, v_permits);

        let last_params = YumaParams::<Test>::new(netuid, universal_pending_emission).unwrap();
        let last_output = YumaEpoch::<Test>::new(netuid, last_params).run().unwrap();
        last_output.clone().apply();
        simulation_result.update(last_output.clone(), tempo, copier_uid, delegation_fee);

        // Write data to CSV
        if let Err(e) = wtr.write_record(&[
            copier_margin.to_string(),
            block_number.to_string(),
            format!("{:?}", last_output.consensus),
            format!("{:?}", last_output.dividends),
            last_output.dividends[copier_uid as usize].to_string(),
            simulation_result.cumulative_copier_divs.to_string(),
            simulation_result.cumulative_avg_delegate_divs.to_string(),
        ]) {
            eprintln!("Error writing CSV row: {}", e);
        }

        if pallet_offworker::is_copying_irrational(simulation_result.clone()) {
            println!(
                "Copying became irrational at block {} with copier margin {}",
                block_number, copier_margin
            );
            break;
        }

        step_block(1);
        iter_counter += 1;

        if iter_counter >= max_epochs {
            println!(
                "Max iterations ({}) reached without copying becoming irrational with copier margin {}",
                max_epochs,
                copier_margin
            );
            break;
        }
    }

    (simulation_result, iter_counter)
}

fn load_json_data() -> Value {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("../snapshots/sn19_weights_stake.json");
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

fn insert_weights_for_block(block_weights: &Value, netuid: u16, permits: Vec<bool>) {
    let uids: Vec<u16> = block_weights
        .as_object()
        .unwrap()
        .keys()
        .filter_map(|uid_str| uid_str.parse::<u16>().ok())
        .collect();

    for uid in uids {
        if let Some(weights) = block_weights[&uid.to_string()].as_array() {
            let weight_vec: Vec<(u16, u16)> = weights
                .iter()
                .filter_map(|w| {
                    let pair = w.as_array()?;
                    Some((pair[0].as_u64()? as u16, pair[1].as_u64()? as u16))
                })
                .collect();

            let (uids, values): (Vec<u16>, Vec<u16>) = weight_vec.into_iter().unzip();
            let zipped_weights: Vec<(u16, u16)> = uids.iter().copied().zip(values).collect();

            if !zipped_weights.is_empty() && zipped_weights.iter().any(|(_, value)| *value != 0) {
                ValidatorPermits::<Test>::insert(netuid, permits.clone());
                Weights::<Test>::insert(netuid, uid, zipped_weights);
            }
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

fn get_validator_permits(block_number: u64, json: &Value) -> Vec<bool> {
    let stuff = json["validator_permits"].as_object().unwrap();
    let block_data = stuff[&block_number.to_string()].as_object().unwrap();

    let mut stuff_vec: Vec<bool> = Vec::new();
    for i in 0..block_data.len() {
        if let Some(value) = block_data.get(&i.to_string()) {
            stuff_vec.push(value.as_bool().unwrap_or(false));
        }
    }

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

    LastUpdate::<Test>::set(netuid, last_update_vec);

    // Overwrite the registration block
    let registration_blocks_vec = get_value_for_block("registration_blocks", block, &json);
    for (i, block) in registration_blocks_vec.iter().enumerate() {
        RegistrationBlock::<Test>::set(netuid, i as u16, *block);
    }
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
        const TEMPO: u64 = 360;

        // Setup the subnet
        register_subnet(u32::MAX, 0).unwrap();
        SubnetConsensusType::<Test>::set(NETUID, Some(SubnetConsensus::Yuma));
        MaxWeightAge::<Test>::insert(NETUID, 100_000);
        Tempo::<Test>::insert(NETUID, TEMPO as u16);
        UseWeightsEncrytyption::<Test>::set(NETUID, false);
        MaxEncryptionPeriod::<Test>::set(NETUID, u64::MAX);

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

        let mut csv_data = Vec::new();

        for iteration in 0..MAX_ITERATIONS {
            // Create parameters out of setup that was made before the loop
            // After first iteration, this will be updated due to dynamically changing weights
            let last_params = YumaParams::<Test>::new(NETUID, UNIVERSAL_PENDING_EMISSION).unwrap();
            let last_output = YumaEpoch::<Test>::new(NETUID, last_params).run().unwrap();
            last_output.clone().apply();

            simulation_result.update(last_output.clone(), TEMPO, COPIER_UID, DELEGATION_FEE);

            csv_data.push((
                TEMPO * iteration as u64,
                last_output.consensus,
                last_output.dividends.clone(),
                last_output.dividends[COPIER_UID as usize],
                simulation_result.cumulative_copier_divs,
                simulation_result.cumulative_avg_delegate_divs,
            ));

            let index_to_change = match iteration % 3 {
                0 => 1, // Second weight
                1 => 3, // Fourth weight
                2 => 5, // Sixth weight
                _ => unreachable!(),
            };

            if let Some(weight) = consensus_weights.get_mut(index_to_change) {
                // Change by 10%
                let change_factor = 1.1; // 10% increase
                *weight = (*weight as f32 * change_factor).round() as u16;
            }

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

        let mut wtr =
            csv::Writer::from_path("simulation_results.csv").expect("Failed to create CSV writer");

        wtr.write_record(&[
            "Block Number",
            "Consensus",
            "All Dividends",
            "Copier Dividend",
            "Cumulative Copier Dividends",
            "Cumulative Average Delegate Dividends",
        ])
        .expect("Failed to write CSV header");

        // Write the data
        for row in csv_data {
            wtr.write_record(&[
                row.0.to_string(),
                format!("{:?}", row.1),
                format!("{:?}", row.2),
                row.3.to_string(),
                row.4.to_string(),
                row.5.to_string(),
            ])
            .expect("Failed to write CSV row");
        }

        wtr.flush().expect("Failed to flush CSV writer");

        // Final check outside the loop
        let is_copy_ira = pallet_offworker::is_copying_irrational(simulation_result.clone());
        assert!(is_copy_ira, "Copying should have become irrational");
    });
}
