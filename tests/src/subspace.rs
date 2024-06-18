use crate::{mock::*, update_params};
use frame_support::{assert_err, assert_noop, assert_ok, pallet_prelude::DispatchError};
use log::info;
use pallet_subnet_consensus::yuma::{AccountKey, EmissionMap, ModuleKey, YumaEpoch};
use pallet_subnet_emission::UnitEmission;
use pallet_subspace::{
    global::BurnConfiguration,
    migrations::v8::old_storage::MinBurn,
    voting::{ApplicationStatus, ProposalData, ProposalStatus, VoteMode},
    Burn, BurnConfig, Curator, CuratorApplications, DaoTreasuryAddress, Dividends, Emission, Error,
    FloorDelegationFee, FloorFounderShare, FounderShare, GeneralSubnetApplicationCost,
    GlobalParams, Incentive, MaxAllowedModules, MaxAllowedSubnets, MaxAllowedUids,
    MaxAllowedWeights, MaxNameLength, MaxRegistrationsPerBlock, MaximumSetWeightCallsPerEpoch,
    MinAllowedWeights, MinNameLength, MinStake, ProfitShares, ProposalCost, ProposalExpiration,
    Proposals, RegistrationsPerBlock, Stake, StakeFrom, StakeTo, SubnetBurn, SubnetBurnConfig,
    SubnetEmission, SubnetGaps, SubnetNames, SubnetParams, SubnetRegistrationsThisInterval, Tempo,
    TotalSubnets, Trust, VoteModeSubnet, N,
};
use sp_core::U256;
use sp_runtime::{DispatchResult, Percent};
use sp_std::vec;
use std::{
    array::from_fn,
    collections::{BTreeMap, BTreeSet},
    marker::PhantomData,
};
use substrate_fixed::types::{extra::U2, I64F64};

// FIXME: get all of the tests back here

// Burn
//=====

// test subnet specific burn
#[test]
fn test_local_subnet_burn() {
    new_test_ext().execute_with(|| {
        let min_burn = to_nano(10);
        let max_burn = to_nano(1000);

        let burn_config = BurnConfiguration {
            min_burn,
            max_burn,
            adjustment_interval: 200,
            expected_registrations: 25,
            ..BurnConfiguration::<Test>::default()
        };

        assert_ok!(burn_config.apply());

        MaxRegistrationsPerBlock::<Test>::set(5);

        // register the general subnet
        assert_ok!(register_module(0, U256::from(0), to_nano(20)));

        // register 500 modules on yuma subnet
        let netuid = 1;
        let n = 300;
        let initial_stake: u64 = to_nano(500);

        MaxRegistrationsPerBlock::<Test>::set(1000);
        // this will perform 300 registrations and step in between
        for i in 1..n {
            // this registers five in block
            assert_ok!(register_module(netuid, U256::from(i), initial_stake));
            if i % 5 == 0 {
                // after that we step 30 blocks
                // meaning that the average registration per block is 0.166..
                step_block(30);
            }
        }

        // We are at block 1,8 k now.
        // We performed 300 registrations
        // this means avg.  0.166.. per block
        // burn has incrased by 90% > up

        let subnet_zero_burn = Burn::<Test>::get(0);
        assert_eq!(subnet_zero_burn, min_burn);
        let subnet_one_burn = Burn::<Test>::get(1);
        assert!(min_burn < subnet_one_burn && subnet_one_burn < max_burn);
    });
}

// Delegate Staking
//=================

#[test]
fn test_ownership_ratio() {
    new_test_ext().execute_with(|| {
        let netuid: u16 = 0;
        let num_modules: u16 = 10;
        let tempo = 1;
        let stake_per_module: u64 = 1_000_000_000;
        // make sure that the results won´t get affected by burn
        zero_min_burn();

        register_n_modules(netuid, num_modules, stake_per_module);
        Tempo::<Test>::insert(netuid, tempo);

        let keys = SubspaceMod::get_keys(netuid);
        let voter_key = keys[0];
        let miner_keys = keys[1..].to_vec();
        let miner_uids: Vec<u16> =
            miner_keys.iter().map(|k| SubspaceMod::get_uid_for_key(netuid, k)).collect();
        let miner_weights = vec![1; miner_uids.len()];

        let delegate_keys: Vec<U256> =
            (0..num_modules).map(|i| U256::from(i + num_modules + 1)).collect();
        for d in delegate_keys.iter() {
            SubspaceMod::add_balance_to_account(&d, stake_per_module + 1);
        }

        let pre_delegate_stake_from_vector = StakeFrom::<Test>::get(&voter_key);
        assert_eq!(pre_delegate_stake_from_vector.len(), 1); // +1 for the module itself, +1 for the delegate key on

        for (i, d) in delegate_keys.iter().enumerate() {
            info!("DELEGATE KEY: {d}");
            assert_ok!(SubspaceMod::add_stake(
                get_origin(*d),
                voter_key,
                stake_per_module
            ));
            let stake_from_vector = StakeFrom::<Test>::get(&voter_key);
            assert_eq!(
                stake_from_vector.len(),
                pre_delegate_stake_from_vector.len() + i + 1
            );
        }
        let ownership_ratios: Vec<(U256, I64F64)> =
            SubnetConsensus::get_ownership_ratios(netuid, &voter_key);
        assert_eq!(ownership_ratios.len(), delegate_keys.len() + 1);

        let founder_tokens_before = SubspaceMod::get_balance(&voter_key)
            + SubspaceMod::get_stake_to_module(&voter_key, &voter_key);

        let delegate_balances_before =
            delegate_keys.iter().map(SubspaceMod::get_balance).collect::<Vec<u64>>();
        let delegate_stakes_before = delegate_keys
            .iter()
            .map(|k| SubspaceMod::get_stake_to_module(k, &voter_key))
            .collect::<Vec<u64>>();
        let delegate_total_tokens_before = delegate_balances_before
            .iter()
            .zip(delegate_stakes_before.clone())
            .map(|(a, x)| a + x)
            .sum::<u64>();

        let total_balance = keys
            .iter()
            .map(SubspaceMod::get_balance)
            .collect::<Vec<u64>>()
            .iter()
            .sum::<u64>();
        let total_stake = keys
            .iter()
            .map(|k| SubspaceMod::get_stake_to_module(k, k))
            .collect::<Vec<u64>>()
            .iter()
            .sum::<u64>();
        let total_delegate_stake = delegate_keys
            .iter()
            .map(|k| SubspaceMod::get_stake_to_module(k, &voter_key))
            .collect::<Vec<u64>>()
            .iter()
            .sum::<u64>();
        let total_delegate_balance = delegate_keys
            .iter()
            .map(SubspaceMod::get_balance)
            .collect::<Vec<u64>>()
            .iter()
            .sum::<u64>();
        let total_tokens_before =
            total_balance + total_stake + total_delegate_stake + total_delegate_balance;
        info!("total_tokens_before: {total_tokens_before:?}");

        info!("delegate_balances before: {delegate_balances_before:?}");
        info!("delegate_stakes before: {delegate_stakes_before:?}");
        info!("delegate_total_tokens before: {delegate_total_tokens_before:?}");

        let result = SubspaceMod::set_weights(
            get_origin(voter_key),
            netuid,
            miner_uids.clone(),
            miner_weights.clone(),
        );

        assert_ok!(result);

        step_epoch(netuid);

        let dividends = Dividends::<Test>::get(netuid);
        let incentives = Incentive::<Test>::get(netuid);
        let emissions = Emission::<Test>::get(netuid);

        info!("dividends: {dividends:?}");
        info!("incentives: {incentives:?}");
        info!("emissions: {emissions:?}");
        let total_emissions = emissions.iter().sum::<u64>();

        info!("total_emissions: {total_emissions:?}");

        let delegate_balances =
            delegate_keys.iter().map(SubspaceMod::get_balance).collect::<Vec<u64>>();
        let delegate_stakes = delegate_keys
            .iter()
            .map(|k| SubspaceMod::get_stake_to_module(k, &voter_key))
            .collect::<Vec<u64>>();
        let delegate_total_tokens = delegate_balances
            .iter()
            .zip(delegate_stakes.clone())
            .map(|(a, x)| a + x)
            .sum::<u64>();
        let founder_tokens = SubspaceMod::get_balance(&voter_key)
            + SubspaceMod::get_stake_to_module(&voter_key, &voter_key);
        let founder_new_tokens = founder_tokens - founder_tokens_before;
        let delegate_new_tokens: Vec<u64> = delegate_stakes
            .iter()
            .zip(delegate_stakes_before.clone())
            .map(|(a, x)| a - x)
            .collect::<Vec<u64>>();

        let total_new_tokens = founder_new_tokens + delegate_new_tokens.iter().sum::<u64>();

        info!("owner_ratios: {ownership_ratios:?}");
        info!("total_new_tokens: {total_new_tokens:?}");
        info!("founder_tokens: {founder_tokens:?}");
        info!("delegate_balances: {delegate_balances:?}");
        info!("delegate_stakes: {delegate_stakes:?}");
        info!("delegate_total_tokens: {delegate_total_tokens:?}");
        info!("founder_new_tokens: {founder_new_tokens:?}");
        info!("delegate_new_tokens: {delegate_new_tokens:?}");

        let total_balance = keys
            .iter()
            .map(SubspaceMod::get_balance)
            .collect::<Vec<u64>>()
            .iter()
            .sum::<u64>();
        let total_stake = keys
            .iter()
            .map(|k| SubspaceMod::get_stake_to_module(k, k))
            .collect::<Vec<u64>>()
            .iter()
            .sum::<u64>();
        let total_delegate_stake = delegate_keys
            .iter()
            .map(|k| SubspaceMod::get_stake_to_module(k, &voter_key))
            .collect::<Vec<u64>>()
            .iter()
            .sum::<u64>();
        let total_delegate_balance = delegate_keys
            .iter()
            .map(SubspaceMod::get_balance)
            .collect::<Vec<u64>>()
            .iter()
            .sum::<u64>();
        let total_tokens_after =
            total_balance + total_stake + total_delegate_stake + total_delegate_balance;
        let total_new_tokens = total_tokens_after - total_tokens_before;
        info!("total_tokens_after: {total_tokens_before:?}");
        info!("total_new_tokens: {total_new_tokens:?}");
        assert_eq!(total_new_tokens, total_emissions);

        let stake_from_vector = StakeFrom::<Test>::get(&voter_key);
        let _stake: u64 = Stake::<Test>::get(&voter_key);
        let _sumed_stake: u64 = stake_from_vector.iter().fold(0, |acc, (_a, x)| acc + x);
        let _total_stake: u64 = SubspaceMod::get_total_subnet_stake(netuid);
        info!("stake_from_vector: {stake_from_vector:?}");
    });
}

// Profit Sharing
//===============

#[test]
fn test_add_profit_share() {
    new_test_ext().execute_with(|| {
        let netuid = 0;
        let miner_key = U256::from(0);
        let voter_key = U256::from(1);
        // make sure that the results won´t get affected by burn
        zero_min_burn();
        FloorFounderShare::<Test>::put(0);

        register_module(netuid, miner_key, 1_000_000_000).expect("register miner module failed");
        register_module(netuid, voter_key, 1_000_000_000).expect("register voter module failed");
        let miner_uid = SubspaceMod::get_uid_for_key(netuid, &miner_key);
        let _voter_uid = SubspaceMod::get_uid_for_key(netuid, &voter_key);

        update_params!(netuid => { min_allowed_weights: 1 });

        let profit_sharer_keys = vec![U256::from(2), U256::from(3)];
        let shares = vec![2, 2];
        let result = SubspaceMod::add_profit_shares(
            get_origin(miner_key),
            profit_sharer_keys.clone(),
            shares.clone(),
        );
        assert_ok!(result);

        let profit_shares = ProfitShares::<Test>::get(miner_key);
        assert_eq!(profit_shares.len(), shares.len(), "profit shares not added");
        info!("founder profit shares: {profit_shares:?}");
        let result =
            SubspaceMod::set_weights(get_origin(voter_key), netuid, vec![miner_uid], vec![1]);

        assert_ok!(result);
        let params = SubspaceMod::subnet_params(netuid);
        info!("params: {params:?}");
        let miner_emission = get_emission_for_key(netuid, &miner_key);
        let voter_emission = get_emission_for_key(netuid, &voter_key);
        assert_eq!(miner_emission, voter_emission, "emission not equal");
        assert!(miner_emission == 0, "emission not equal");
        assert!(voter_emission == 0, "emission not equal");
        let miner_stake = Stake::<Test>::get(miner_key);
        let voter_stake = Stake::<Test>::get(voter_key);
        info!("miner stake before: {miner_stake:?}");
        info!("voter stake before: {voter_stake:?}");
        step_epoch(netuid);
        let miner_emission = get_emission_for_key(netuid, &miner_key);
        let voter_emission = get_emission_for_key(netuid, &voter_key);
        assert!(miner_emission > 0, "emission not equal");
        assert!(voter_emission > 0, "emission not equal");
        assert_eq!(miner_emission, voter_emission, "emission not equal");

        info!("miner emission: {miner_emission:?}");
        info!("voter emission: {voter_emission:?}");
        let miner_balance = SubspaceMod::get_balance_u64(&miner_key);
        let voter_balance = SubspaceMod::get_balance_u64(&voter_key);
        info!("miner balance: {miner_balance:?}");
        info!("voter balance: {voter_balance:?}");
        let miner_stake = Stake::<Test>::get(miner_key);
        let voter_stake = Stake::<Test>::get(voter_key);
        info!("miner stake after: {miner_stake:?}");
        info!("voter stake after: {voter_stake:?}");

        let profit_share_emissions = ProfitShares::<Test>::get(miner_key);
        info!("profit share emissions: {profit_share_emissions:?}");

        // check the profit sharers
        let mut profit_share_balances: Vec<u64> = Vec::new();
        for profit_sharer_key in profit_sharer_keys.iter() {
            let profit_share_balance = StakeTo::<Test>::get(profit_sharer_key).into_values().sum();
            let stake_to_vector = StakeTo::<Test>::get(profit_sharer_key);
            info!("profit share balance: {stake_to_vector:?}");
            info!("profit share balance: {profit_share_balance:?}");
            profit_share_balances.push(profit_share_balance);
            assert!(profit_share_balances[0] > 0, "profit share balance is zero");
        }

        // sum of profit shares should be equal to the emission
        let sum_profit_share_balances: u64 = profit_share_balances.iter().sum();
        let delta = 1000;
        assert!(
            sum_profit_share_balances > miner_emission - delta
                || sum_profit_share_balances < miner_emission + delta,
            "profit share balances do not add up to the emission"
        );
    })
}

// Registraions
//=============

#[test]
#[ignore = "global stake ?"]
fn test_min_stake() {
    new_test_ext().execute_with(|| {
        let netuid: u16 = 0;
        let min_stake = 100_000_000;
        let max_registrations_per_block = 10;
        let reg_this_block: u16 = 100;
        // make sure that the results won´t get affected by burn
        zero_min_burn();
        let mut network: Vec<u8> = "test".as_bytes().to_vec();
        let address: Vec<u8> = "0.0.0.0:30333".as_bytes().to_vec();
        network.extend(netuid.to_string().as_bytes().to_vec());
        let name = "module".as_bytes().to_vec();
        assert_noop!(SubspaceMod::do_register(get_origin(U256::from(0)), network, name, address, 0, U256::from(0), None), Error::<Test>::NotEnoughBalanceToRegister);
        MinStake::<Test>::set(netuid, min_stake);
        MaxRegistrationsPerBlock::<Test>::set(max_registrations_per_block);
        step_block(1);
        assert_eq!(RegistrationsPerBlock::<Test>::get(), 0);

        let n = U256::from(reg_this_block);
        let keys_list: Vec<U256> = (1..n.as_u64())
            .map(U256::from)
            .collect();

        let min_stake_to_register = MinStake::<Test>::get(netuid);

        for key in keys_list {
            let _ = register_module(netuid, key, min_stake_to_register);
            info!("registered module with key: {key:?} and min_stake_to_register: {min_stake_to_register:?}");
        }
        let registrations_this_block = RegistrationsPerBlock::<Test>::get();
        info!("registrations_this_block: {registrations_this_block:?}");
        assert_eq!(registrations_this_block, max_registrations_per_block);

        step_block(1);
        assert_eq!(RegistrationsPerBlock::<Test>::get(), 0);
    });
}

#[test]
fn test_max_registration() {
    new_test_ext().execute_with(|| {
        let netuid: u16 = 0;
        let rounds = 3;
        let max_registrations_per_block = 100;
        // make sure that the results won´t get affected by burn
        zero_min_burn();

        assert_eq!(RegistrationsPerBlock::<Test>::get(), 0);

        MaxRegistrationsPerBlock::<Test>::set(1000);
        for i in 1..(max_registrations_per_block * rounds) {
            let key = U256::from(i);
            assert_ok!(register_module(netuid, U256::from(i), to_nano(100)));
            let registrations_this_block = RegistrationsPerBlock::<Test>::get();
            assert_eq!(registrations_this_block, i);
            assert!(SubspaceMod::is_registered(None, &key));
        }
        step_block(1);
        assert_eq!(RegistrationsPerBlock::<Test>::get(), 0);
    });
}

#[test]
fn test_delegate_register() {
    new_test_ext().execute_with(|| {
        let netuid: u16 = 0;
        let n: u16 = 10;
        let key: U256 = U256::from(n + 1);
        let module_keys: Vec<U256> = (0..n).map(U256::from).collect();
        let stake_amount: u64 = 10_000_000_000;
        // make sure that the results won´t get affected by burn
        zero_min_burn();

        SubspaceMod::add_balance_to_account(&key, stake_amount * n as u64);
        for module_key in module_keys {
            delegate_register_module(netuid, key, module_key, stake_amount)
                .expect("delegate register module failed");
            let key_balance = SubspaceMod::get_balance_u64(&key);
            let stake_to_module = SubspaceMod::get_stake_to_module(&key, &module_key);
            info!("key_balance: {key_balance:?}");
            let stake_to_vector = Stake::<Test>::get(&key);
            info!("stake_to_vector: {stake_to_vector:?}");
            assert_eq!(stake_to_module, stake_amount);
        }
    });
}

#[test]
fn test_registration_ok() {
    new_test_ext().execute_with(|| {
        let netuid: u16 = 0;
        let key: U256 = U256::from(1);
        // make sure that the results won´t get affected by burn
        zero_min_burn();

        // make sure there is some balance
        SubspaceMod::add_balance_to_account(&key, 2);
        register_module(netuid, key, 1)
            .unwrap_or_else(|_| panic!("register module failed for key {key:?}"));

        // Check if module has added to the specified network(netuid)
        assert_eq!(N::<Test>::get(netuid), 1);

        // Check if the module has added to the Keys
        let module_uid = SubspaceMod::get_uid_for_key(netuid, &key);
        assert_eq!(SubspaceMod::get_uid_for_key(netuid, &key), 0);
        // Check if module has added to Uids
        let neuro_uid = SubspaceMod::get_uid_for_key(netuid, &key);
        assert_eq!(neuro_uid, module_uid);

        // Check if the balance of this hotkey account for this subnetwork == 0
        assert_eq!(get_stake_for_uid(netuid, module_uid), 1);
    });
}

#[test]
fn test_many_registrations() {
    new_test_ext().execute_with(|| {
        let netuid = 0;
        let stake = 10;
        let n = 100;
        // make sure that the results won´t get affected by burn
        zero_min_burn();

        MaxRegistrationsPerBlock::<Test>::set(n);
        for i in 0..n {
            register_module(netuid, U256::from(i), stake).unwrap_or_else(|_| {
                panic!("Failed to register module with key: {i:?} and stake: {stake:?}",)
            });
            assert_eq!(N::<Test>::get(netuid), i + 1, "Failed at i={i}",);
        }
    });
}

#[test]
fn test_registration_with_stake() {
    new_test_ext().execute_with(|| {
        let netuid = 0;
        let stake_vector: Vec<u64> = [100000, 1000000, 10000000].to_vec();

        // make sure that the results won´t get affected by burn
        zero_min_burn();

        for (i, stake) in stake_vector.iter().enumerate() {
            let uid: u16 = i as u16;
            let stake_value: u64 = *stake;

            let key = U256::from(uid);
            info!("key: {key:?}");
            info!("stake: {stake_value:?}");
            let stake_before: u64 = Stake::<Test>::get(&key);
            info!("stake_before: {stake_before:?}");
            register_module(netuid, key, stake_value).unwrap_or_else(|_| {
                panic!("Failed to register module with key: {key:?} and stake: {stake_value:?}",)
            });
            info!("balance: {:?}", SubspaceMod::get_balance_u64(&key));
            assert_eq!(get_stake_for_uid(netuid, uid), stake_value);
        }
    });
}

#[test]
fn register_same_key_twice() {
    new_test_ext().execute_with(|| {
        let netuid = 0;
        let stake = 10;
        let key = U256::from(1);
        // make sure that the results won´t get affected by burn
        zero_min_burn();

        assert_ok!(register_module(netuid, key, stake));
        assert_err!(
            register_module(netuid, key, stake),
            Error::<Test>::KeyAlreadyRegistered
        );
    });
}

#[test]
fn test_whitelist() {
    new_test_ext().execute_with(|| {
        let key = U256::from(0);
        let adding_key = U256::from(1);
        let mut params = SubspaceMod::global_params();
        params.curator = key;
        SubspaceMod::set_global_params(params);

        let proposal_cost = GeneralSubnetApplicationCost::<Test>::get();
        let data = "test".as_bytes().to_vec();

        SubspaceMod::add_balance_to_account(&key, proposal_cost + 1);
        // first submit an application
        let balance_before = SubspaceMod::get_balance_u64(&key);

        assert_ok!(SubspaceMod::add_dao_application(
            get_origin(key),
            adding_key,
            data.clone(),
        ));

        let balance_after = SubspaceMod::get_balance_u64(&key);

        dbg!(balance_before, balance_after);
        assert_eq!(balance_after, balance_before - proposal_cost);

        // Assert that the proposal is initially in the Pending status
        for (_, value) in CuratorApplications::<Test>::iter() {
            assert_eq!(value.status, ApplicationStatus::Pending);
            assert_eq!(value.user_id, adding_key);
            assert_eq!(value.data, data);
        }

        // add key to whitelist
        assert_ok!(SubspaceMod::add_to_whitelist(
            get_origin(key),
            adding_key,
            1,
        ));

        let balance_after_accept = SubspaceMod::get_balance_u64(&key);

        assert_eq!(balance_after_accept, balance_before);

        // Assert that the proposal is now in the Accepted status
        for (_, value) in CuratorApplications::<Test>::iter() {
            assert_eq!(value.status, ApplicationStatus::Accepted);
            assert_eq!(value.user_id, adding_key);
            assert_eq!(value.data, data);
        }

        assert!(SubspaceMod::is_in_legit_whitelist(&adding_key));
    });
}

fn register_custom(netuid: u16, key: U256, name: &[u8], addr: &[u8]) -> DispatchResult {
    let network: Vec<u8> = format!("test{netuid}").as_bytes().to_vec();

    let origin = get_origin(key);
    let is_new_subnet: bool = !SubspaceMod::if_subnet_exist(netuid);
    if is_new_subnet {
        MaxRegistrationsPerBlock::<Test>::set(1000)
    }

    // make sure there is some balance
    SubspaceMod::add_balance_to_account(&key, 2);
    SubspaceMod::register(origin, network, name.to_vec(), addr.to_vec(), 1, key, None)
}

fn test_validation_cases(f: impl Fn(&[u8], &[u8]) -> DispatchResult) {
    assert_err!(f(b"", b""), Error::<Test>::InvalidModuleName);
    assert_err!(
        f("o".repeat(100).as_bytes(), b""),
        Error::<Test>::ModuleNameTooLong
    );
    assert_err!(f(b"\xc3\x28", b""), Error::<Test>::InvalidModuleName);

    assert_err!(f(b"test", b""), Error::<Test>::InvalidModuleAddress);
    assert_err!(
        f(b"test", "o".repeat(100).as_bytes()),
        Error::<Test>::ModuleAddressTooLong
    );
    assert_err!(f(b"test", b"\xc3\x28"), Error::<Test>::InvalidModuleAddress);

    assert_ok!(f(b"test", b"abc"));
}

#[test]
fn validates_module_on_registration() {
    new_test_ext().execute_with(|| {
        // make sure that the results won´t get affected by burn
        zero_min_burn();
        test_validation_cases(|name, addr| register_custom(0, 0.into(), name, addr));

        assert_err!(
            register_custom(0, 1.into(), b"test", b"0.0.0.0:1"),
            Error::<Test>::ModuleNameAlreadyExists
        );
    });
}

#[test]
fn validates_module_on_update() {
    new_test_ext().execute_with(|| {
        let subnet = 0;
        let key_0: U256 = 0.into();
        let origin_0 = get_origin(0.into());
        // make sure that the results won´t get affected by burn
        zero_min_burn();

        assert_ok!(register_custom(subnet, key_0, b"test", b"0.0.0.0:1"));

        test_validation_cases(|name, addr| {
            SubspaceMod::update_module(
                origin_0.clone(),
                subnet,
                name.to_vec(),
                addr.to_vec(),
                None,
                None,
            )
        });

        let key_1: U256 = 1.into();
        let origin_1 = get_origin(key_1);
        assert_ok!(register_custom(0, key_1, b"test2", b"0.0.0.0:2"));

        let update_module = |name: &[u8], addr: &[u8]| {
            SubspaceMod::update_module(
                origin_1.clone(),
                subnet,
                name.to_vec(),
                addr.to_vec(),
                Some(Percent::from_percent(5)),
                None,
            )
        };

        assert_err!(
            update_module(b"test", b""),
            Error::<Test>::ModuleNameAlreadyExists
        );
        assert_ok!(update_module(b"test2", b"0.0.0.0:2"));
        assert_ok!(update_module(b"test3", b"0.0.0.0:3"));

        let params = SubspaceMod::module_params(0, &key_1);
        assert_eq!(params.name, b"test3");
        assert_eq!(params.address, b"0.0.0.0:3");

        FloorDelegationFee::<Test>::put(Percent::from_percent(10));
        assert_err!(
            update_module(b"test3", b"0.0.0.0:3"),
            Error::<Test>::InvalidMinDelegationFee
        );
    });
}

#[test]
fn deregister_within_subnet_when_limit_is_reached() {
    new_test_ext().execute_with(|| {
        MaxAllowedModules::<Test>::set(3);

        assert_ok!(register_module(0, 0.into(), to_nano(10_000)));
        assert_ok!(register_module(1, 1.into(), to_nano(5_000)));

        assert_eq!(Stake::<Test>::get(U256::from(0)), to_nano(9_996));
        assert_eq!(Stake::<Test>::get(U256::from(1)), to_nano(4_996));

        MaxAllowedUids::<Test>::set(0, 1);
        MaxAllowedUids::<Test>::set(1, 1);

        assert_ok!(register_module(0, 2.into(), to_nano(15_000)));

        assert_eq!(Stake::<Test>::get(U256::from(2)), to_nano(14_996));
        assert_eq!(Stake::<Test>::get(U256::from(1)), to_nano(4_996));

        assert_eq!(Emission::<Test>::get(0).len(), 1);
        assert_eq!(Emission::<Test>::get(1).len(), 1);
    });
}

#[test]
#[ignore = "global stake ?"]
fn deregister_globally_when_global_limit_is_reached() {
    new_test_ext().execute_with(|| {
        MaxAllowedModules::<Test>::set(2);

        assert_ok!(register_module(0, 0.into(), to_nano(10_000)));
        assert_ok!(register_module(1, 1.into(), to_nano(5_000)));

        assert_eq!(Stake::<Test>::get(U256::from(0)), to_nano(9_996));
        assert_eq!(Stake::<Test>::get(U256::from(1)), to_nano(4_996));

        MaxAllowedUids::<Test>::set(0, 2);
        MaxAllowedUids::<Test>::set(1, 1);

        assert_ok!(register_module(0, 2.into(), to_nano(15_000)));

        assert_eq!(Stake::<Test>::get(U256::from(0)), to_nano(9_996));
        assert_eq!(Stake::<Test>::get(U256::from(2)), to_nano(14_996));
        assert_eq!(Stake::<Test>::get(U256::from(1)), 0);

        assert_eq!(Emission::<Test>::get(0).len(), 2);
        assert_eq!(Emission::<Test>::get(1).len(), 0);
    });
}

// Names
#[test]
fn test_register_invalid_name() {
    new_test_ext().execute_with(|| {
        let network_name = b"testnet".to_vec();
        let address = b"0x1234567890".to_vec();
        let stake = to_nano(1);

        // make registrations free
        zero_min_burn();

        // set min name lenght
        MinNameLength::<Test>::put(2);

        // Get the minimum and maximum name lengths from the configuration
        let min_name_length = MinNameLength::<Test>::get();
        let max_name_length = MaxNameLength::<Test>::get();

        // Try registering with an empty name (invalid)
        let empty_name = Vec::new();
        let register_one = U256::from(0);
        SubspaceMod::add_balance_to_account(&register_one, stake + 1);

        assert_noop!(
            SubspaceMod::register(
                get_origin(register_one),
                network_name.clone(),
                empty_name,
                address.clone(),
                stake,
                register_one,
                None,
            ),
            Error::<Test>::InvalidModuleName
        );

        // Try registering with a name that is too short (invalid)
        let register_two = U256::from(1);
        let short_name = b"a".to_vec();
        SubspaceMod::add_balance_to_account(&register_two, stake + 1);
        assert_noop!(
            SubspaceMod::register(
                get_origin(register_two),
                network_name.clone(),
                short_name,
                address.clone(),
                stake,
                register_two,
                None,
            ),
            Error::<Test>::ModuleNameTooShort
        );

        // Try registering with a name that is exactly the minimum length (valid)
        let register_three = U256::from(2);
        let min_length_name = vec![b'a'; min_name_length as usize];
        SubspaceMod::add_balance_to_account(&register_three, stake + 1);
        assert_ok!(SubspaceMod::register(
            get_origin(register_three),
            network_name.clone(),
            min_length_name,
            address.clone(),
            stake,
            register_three,
            None,
        ));

        // Try registering with a name that is exactly the maximum length (valid)
        let max_length_name = vec![b'a'; max_name_length as usize];
        let register_four = U256::from(3);
        SubspaceMod::add_balance_to_account(&register_four, stake + 1);
        assert_ok!(SubspaceMod::register(
            get_origin(register_four),
            network_name.clone(),
            max_length_name,
            address.clone(),
            stake,
            register_four,
            None,
        ));

        // Try registering with a name that is too long (invalid)
        let long_name = vec![b'a'; (max_name_length + 1) as usize];
        let register_five = U256::from(4);
        SubspaceMod::add_balance_to_account(&register_five, stake + 1);
        assert_noop!(
            SubspaceMod::register(
                get_origin(register_five),
                network_name,
                long_name,
                address,
                stake,
                register_five,
                None,
            ),
            Error::<Test>::ModuleNameTooLong
        );
    });
}

#[test]
fn test_register_invalid_subnet_name() {
    new_test_ext().execute_with(|| {
        let address = b"0x1234567890".to_vec();
        let stake = to_nano(1);
        let module_name = b"test".to_vec();

        // Make registrations free
        zero_min_burn();

        // Set min name length
        MinNameLength::<Test>::put(2);

        // Get the minimum and maximum name lengths from the configuration
        let min_name_length = MinNameLength::<Test>::get();
        let max_name_length = MaxNameLength::<Test>::get();

        let register_one = U256::from(0);
        let empty_name = Vec::new();
        SubspaceMod::add_balance_to_account(&register_one, stake + 1);
        assert_noop!(
            SubspaceMod::register(
                get_origin(register_one),
                empty_name,
                module_name.clone(),
                address.clone(),
                stake,
                register_one,
                None,
            ),
            Error::<Test>::InvalidSubnetName
        );

        // Try registering with a name that is too short (invalid)
        let register_two = U256::from(1);
        let short_name = b"a".to_vec();
        SubspaceMod::add_balance_to_account(&register_two, stake + 1);
        assert_noop!(
            SubspaceMod::register(
                get_origin(register_two),
                short_name,
                module_name.clone(),
                address.clone(),
                stake,
                register_two,
                None,
            ),
            Error::<Test>::SubnetNameTooShort
        );

        // Try registering with a name that is exactly the minimum length (valid)
        let register_three = U256::from(2);
        let min_length_name = vec![b'a'; min_name_length as usize];
        SubspaceMod::add_balance_to_account(&register_three, stake + 1);
        assert_ok!(SubspaceMod::register(
            get_origin(register_three),
            min_length_name,
            module_name.clone(),
            address.clone(),
            stake,
            register_three,
            None,
        ));

        // Try registering with a name that is exactly the maximum length (valid)
        let max_length_name = vec![b'a'; max_name_length as usize];
        let register_four = U256::from(3);
        SubspaceMod::add_balance_to_account(&register_four, stake + 1);
        assert_ok!(SubspaceMod::register(
            get_origin(register_four),
            max_length_name,
            module_name.clone(),
            address.clone(),
            stake,
            register_four,
            None,
        ));

        // Try registering with a name that is too long (invalid)
        let long_name = vec![b'a'; (max_name_length + 1) as usize];
        let register_five = U256::from(4);
        SubspaceMod::add_balance_to_account(&register_five, stake + 1);
        assert_noop!(
            SubspaceMod::register(
                get_origin(register_five),
                long_name,
                module_name.clone(),
                address.clone(),
                stake,
                register_five,
                None,
            ),
            Error::<Test>::SubnetNameTooLong
        );

        // Try registering with an invalid UTF-8 name (invalid)
        let invalid_utf8_name = vec![0xFF, 0xFF];
        let register_six = U256::from(5);
        SubspaceMod::add_balance_to_account(&register_six, stake + 1);
        assert_noop!(
            SubspaceMod::register(
                get_origin(register_six),
                invalid_utf8_name,
                module_name.clone(),
                address,
                stake,
                register_six,
                None,
            ),
            Error::<Test>::InvalidSubnetName
        );
    });
}

// Subnet 0 Whitelist
#[test]
fn test_remove_from_whitelist() {
    new_test_ext().execute_with(|| {
        let whitelist_key = U256::from(0);
        let module_key = U256::from(1);
        Curator::<Test>::put(whitelist_key);

        let proposal_cost = ProposalCost::<Test>::get();
        let data = "test".as_bytes().to_vec();

        // apply
        SubspaceMod::add_balance_to_account(&whitelist_key, proposal_cost + 1);
        // first submit an application
        assert_ok!(SubspaceMod::add_dao_application(
            get_origin(whitelist_key),
            module_key,
            data.clone(),
        ));

        // Add the module_key to the whitelist
        assert_ok!(SubspaceMod::add_to_whitelist(
            get_origin(whitelist_key),
            module_key,
            1
        ));
        assert!(SubspaceMod::is_in_legit_whitelist(&module_key));

        // Remove the module_key from the whitelist
        assert_ok!(SubspaceMod::remove_from_whitelist(
            get_origin(whitelist_key),
            module_key
        ));
        assert!(!SubspaceMod::is_in_legit_whitelist(&module_key));
    });
}

#[test]
fn test_invalid_curator() {
    new_test_ext().execute_with(|| {
        let whitelist_key = U256::from(0);
        let invalid_key = U256::from(1);
        let module_key = U256::from(2);
        Curator::<Test>::put(whitelist_key);

        // Try to add to whitelist with an invalid curator key
        assert_noop!(
            SubspaceMod::add_to_whitelist(get_origin(invalid_key), module_key, 1),
            Error::<Test>::NotCurator
        );
        assert!(!SubspaceMod::is_in_legit_whitelist(&module_key));
    });
}

#[test]
fn new_subnet_reutilized_removed_netuid_if_total_is_bigger_than_removed() {
    new_test_ext().execute_with(|| {
        zero_min_burn();

        TotalSubnets::<Test>::set(10);
        SubnetGaps::<Test>::set(BTreeSet::from([5]));
        assert_ok!(register_module(0, 0.into(), to_nano(1)));

        let subnets: Vec<_> = N::<Test>::iter().collect();
        assert_eq!(subnets, vec![(5, 1)]);
        assert_eq!(SubnetGaps::<Test>::get(), BTreeSet::from([]));
    });
}

#[test]
fn new_subnet_does_not_reute_removed_netuid_if_total_is_smaller_than_removed() {
    new_test_ext().execute_with(|| {
        zero_min_burn();

        TotalSubnets::<Test>::set(3);
        SubnetGaps::<Test>::set(BTreeSet::from([7]));
        assert_ok!(register_module(0, 0.into(), to_nano(1)));

        let subnets: Vec<_> = N::<Test>::iter().collect();
        assert_eq!(subnets, vec![(7, 1)]);
        assert_eq!(SubnetGaps::<Test>::get(), BTreeSet::from([]));
    });
}

#[test]
#[ignore = "global stake ?"]
fn new_subnets_on_removed_uids_register_modules_to_the_correct_netuids() {
    fn assert_subnets(v: &[(u16, &str)]) {
        let v: Vec<_> = v.iter().map(|(u, n)| (*u, n.as_bytes().to_vec())).collect();
        let names: Vec<_> = SubnetNames::<Test>::iter().collect();
        assert_eq!(names, v);
    }

    new_test_ext().execute_with(|| {
        zero_min_burn();
        MaxAllowedSubnets::<Test>::put(3);

        assert_ok!(register_module(0, 0.into(), to_nano(10)));
        assert_ok!(register_module(1, 1.into(), to_nano(5)));
        assert_ok!(register_module(2, 2.into(), to_nano(1)));
        assert_subnets(&[(0, "test0"), (1, "test1"), (2, "test2")]);

        assert_ok!(register_module(3, 3.into(), to_nano(15)));
        assert_subnets(&[(0, "test0"), (1, "test1"), (2, "test3")]);

        assert_ok!(register_module(4, 4.into(), to_nano(20)));
        assert_subnets(&[(0, "test0"), (1, "test4"), (2, "test3")]);

        SubspaceMod::add_balance_to_account(&0.into(), to_nano(50));
        assert_ok!(SubspaceMod::add_stake(
            get_origin(0.into()),
            0.into(),
            to_nano(10)
        ));

        assert_ok!(register_module(5, 5.into(), to_nano(17)));
        assert_subnets(&[(0, "test0"), (1, "test4"), (2, "test5")]);

        assert_eq!(
            Stake::<Test>::iter_keys().filter(|k| k == &0.into()).count(),
            1
        );
        assert_eq!(
            Stake::<Test>::iter_keys().filter(|k| k == &1.into()).count(),
            1
        );
        assert_eq!(
            Stake::<Test>::iter_keys().filter(|k| k == &2.into()).count(),
            1
        );

        assert_eq!(N::<Test>::get(0), 1);
        assert_eq!(N::<Test>::get(1), 1);
        assert_eq!(N::<Test>::get(2), 1);
    });
}

// Staking
//========

#[test]
fn test_stake() {
    new_test_ext().execute_with(|| {
        let max_uids: u16 = 10;
        let netuids: [u16; 4] = core::array::from_fn(|i| i as u16);
        let amount_staked_vector: Vec<u64> = netuids.iter().map(|_| to_nano(10)).collect();
        let mut total_stake: u64 = 0;
        let mut subnet_stake: u64 = 0;
        // make sure that the results won´t get affected by burn
        zero_min_burn();
        MaxRegistrationsPerBlock::<Test>::set(1000);

        for netuid in netuids {
            info!("NETUID: {}", netuid);
            let amount_staked = amount_staked_vector[netuid as usize];
            let key_vector: Vec<U256> =
                (0..max_uids).map(|i| U256::from(i + max_uids * netuid)).collect();

            for key in key_vector.iter() {
                info!(
                    " KEY {} KEY STAKE {} STAKING AMOUNT {} ",
                    key,
                    Stake::<Test>::get(key),
                    amount_staked
                );

                assert_ok!(register_module(netuid, *key, amount_staked));
                info!(
                    " KEY STAKE {} STAKING AMOUNT {} ",
                    Stake::<Test>::get(key),
                    amount_staked
                );

                // SubspaceMod::add_stake(get_origin(*key), netuid, amount_staked);
                assert_eq!(Stake::<Test>::get(key), amount_staked);
                assert_eq!(SubspaceMod::get_balance(key), 1);

                // REMOVE STAKE
                assert_ok!(SubspaceMod::remove_stake(
                    get_origin(*key),
                    *key,
                    amount_staked
                ));
                assert_eq!(SubspaceMod::get_balance(key), amount_staked + 1);
                assert_eq!(Stake::<Test>::get(key), 0);

                // ADD STAKE AGAIN LOL
                assert_ok!(SubspaceMod::add_stake(
                    get_origin(*key),
                    *key,
                    amount_staked
                ));
                assert_eq!(Stake::<Test>::get(key), amount_staked);
                assert_eq!(SubspaceMod::get_balance(key), 1);

                // AT THE END WE SHOULD HAVE THE SAME TOTAL STAKE
                subnet_stake += Stake::<Test>::get(key);
            }
            assert_eq!(SubspaceMod::get_total_subnet_stake(netuid), subnet_stake);
            total_stake += subnet_stake;
            assert_eq!(SubspaceMod::total_stake(), total_stake);
            subnet_stake = 0;
            info!("TOTAL STAKE: {}", total_stake);
            info!(
                "TOTAL SUBNET STAKE: {}",
                SubspaceMod::get_total_subnet_stake(netuid)
            );
        }
    });
}

#[test]
fn test_multiple_stake() {
    new_test_ext().execute_with(|| {
        let n: u16 = 10;
        let stake_amount: u64 = 10_000_000_000;
        let _total_stake: u64 = 0;
        let netuid: u16 = 0;
        let _subnet_stake: u64 = 0;
        let _uid: u16 = 0;
        let num_staked_modules: u16 = 10;
        let total_stake: u64 = stake_amount * num_staked_modules as u64;
        // make sure that the results won´t get affected by burn
        zero_min_burn();

        register_n_modules(netuid, n, 10);
        let controler_key = U256::from(n + 1);
        let og_staker_balance: u64 = total_stake + 1;
        SubspaceMod::add_balance_to_account(&controler_key, og_staker_balance);

        let keys: Vec<U256> = SubspaceMod::get_keys(netuid);

        // stake to all modules

        let stake_amounts: Vec<u64> = vec![stake_amount; num_staked_modules as usize];

        info!("STAKE AMOUNTS: {stake_amounts:?}");
        let total_actual_stake: u64 =
            keys.clone().into_iter().map(|k| Stake::<Test>::get(&k)).sum();
        let staker_balance = SubspaceMod::get_balance(&controler_key);
        info!("TOTAL ACTUAL STAKE: {total_actual_stake}");
        info!("TOTAL STAKE: {total_stake}");
        info!("STAKER BALANCE: {staker_balance}");
        assert_ok!(SubspaceMod::add_stake_multiple(
            get_origin(controler_key),
            keys.clone(),
            stake_amounts.clone(),
        ));

        let total_actual_stake: u64 =
            keys.clone().into_iter().map(|k| Stake::<Test>::get(&k)).sum();
        let staker_balance = SubspaceMod::get_balance(&controler_key);

        assert_eq!(
            total_actual_stake,
            total_stake + (n as u64 * 10),
            "total stake should be equal to the sum of all stakes"
        );
        assert_eq!(
            staker_balance,
            og_staker_balance - total_stake,
            "staker balance should be 0"
        );

        // unstake from all modules
        assert_ok!(SubspaceMod::remove_stake_multiple(
            get_origin(controler_key),
            keys.clone(),
            stake_amounts.clone(),
        ));

        let total_actual_stake: u64 =
            keys.clone().into_iter().map(|k| Stake::<Test>::get(&k)).sum();
        let staker_balance = SubspaceMod::get_balance(&controler_key);
        assert_eq!(
            total_actual_stake,
            n as u64 * 10,
            "total stake should be equal to the sum of all stakes"
        );
        assert_eq!(
            staker_balance, og_staker_balance,
            "staker balance should be 0"
        );
    });
}

#[test]
fn test_transfer_stake() {
    new_test_ext().execute_with(|| {
        let n: u16 = 10;
        let stake_amount: u64 = 10_000_000_000;
        let netuid: u16 = 0;
        zero_min_burn();

        register_n_modules(netuid, n, stake_amount);

        let keys: Vec<U256> = SubspaceMod::get_keys(netuid);

        assert_ok!(SubspaceMod::transfer_stake(
            get_origin(keys[0]),
            keys[0],
            keys[1],
            stake_amount
        ));

        let key0_stake = Stake::<Test>::get(&keys[0]);
        let key1_stake = Stake::<Test>::get(&keys[1]);
        assert_eq!(key0_stake, 0);
        assert_eq!(key1_stake, stake_amount * 2);

        assert_ok!(SubspaceMod::transfer_stake(
            get_origin(keys[0]),
            keys[1],
            keys[0],
            stake_amount
        ));

        let key0_stake = Stake::<Test>::get(&keys[0]);
        let key1_stake = Stake::<Test>::get(&keys[1]);
        assert_eq!(key0_stake, stake_amount);
        assert_eq!(key1_stake, stake_amount);
    });
}

#[test]
fn test_delegate_stake() {
    new_test_ext().execute_with(|| {
        let max_uids: u16 = 10;
        let netuids: Vec<u16> = [0, 1, 2, 3].to_vec();
        let amount_staked_vector: Vec<u64> = netuids.iter().map(|_i| to_nano(10)).collect();
        let mut total_stake: u64 = 0;
        let mut subnet_stake: u64 = 0;
        // make sure that the results won´t get affected by burn
        zero_min_burn();
        MaxRegistrationsPerBlock::<Test>::set(1000);

        for i in netuids.iter() {
            let netuid = *i;
            info!("NETUID: {}", netuid);
            let amount_staked = amount_staked_vector[netuid as usize];
            let key_vector: Vec<U256> =
                (0..max_uids).map(|i| U256::from(i + max_uids * netuid)).collect();
            let delegate_key_vector: Vec<U256> = key_vector.iter().map(|i| (*i + 1)).collect();

            for (i, key) in key_vector.iter().enumerate() {
                info!(
                    " KEY {} KEY STAKE {} STAKING AMOUNT {} ",
                    key,
                    Stake::<Test>::get(key),
                    amount_staked
                );

                let delegate_key: U256 = delegate_key_vector[i];
                SubspaceMod::add_balance_to_account(&delegate_key, amount_staked + 1);

                assert_ok!(register_module(netuid, *key, 10));
                info!(
                    " DELEGATE KEY STAKE {} STAKING AMOUNT {} ",
                    Stake::<Test>::get(&delegate_key),
                    amount_staked
                );

                assert_ok!(SubspaceMod::add_stake(
                    get_origin(delegate_key),
                    *key,
                    amount_staked
                ));
                let uid = SubspaceMod::get_uid_for_key(netuid, key);
                // SubspaceMod::add_stake(get_origin(*key), netuid, amount_staked);
                assert_eq!(get_stake_for_uid(netuid, uid), amount_staked + 10);
                assert_eq!(SubspaceMod::get_balance(&delegate_key), 1);
                assert_eq!(StakeTo::<Test>::get(&delegate_key).len(), 1);
                // REMOVE STAKE
                assert_ok!(SubspaceMod::remove_stake(
                    get_origin(delegate_key),
                    *key,
                    amount_staked
                ));
                assert_eq!(SubspaceMod::get_balance(&delegate_key), amount_staked + 1);
                assert_eq!(get_stake_for_uid(netuid, uid), 10);
                assert_eq!(StakeTo::<Test>::get(&delegate_key).len(), 0);

                // ADD STAKE AGAIN
                assert_ok!(SubspaceMod::add_stake(
                    get_origin(delegate_key),
                    *key,
                    amount_staked
                ));
                assert_eq!(get_stake_for_uid(netuid, uid), amount_staked + 10);
                assert_eq!(SubspaceMod::get_balance(&delegate_key), 1);
                assert_eq!(StakeTo::<Test>::get(&delegate_key).len(), 1);

                // AT THE END WE SHOULD HAVE THE SAME TOTAL STAKE
                subnet_stake += get_stake_for_uid(netuid, uid);
            }
            assert_eq!(SubspaceMod::get_total_subnet_stake(netuid), subnet_stake);
            total_stake += subnet_stake;
            assert_eq!(SubspaceMod::total_stake(), total_stake);
            subnet_stake = 0;
            info!("TOTAL STAKE: {}", total_stake);
            info!(
                "TOTAL SUBNET STAKE: {}",
                SubspaceMod::get_total_subnet_stake(netuid)
            );
        }
    });
}

#[test]
fn test_ownership_ratio_v2() {
    new_test_ext().execute_with(|| {
        let netuid: u16 = 0;
        let num_modules: u16 = 10;
        let stake_per_module: u64 = 1_000_000_000;
        // make sure that the results won´t get affected by burn
        zero_min_burn();

        register_n_modules(netuid, num_modules, 10);

        let keys = SubspaceMod::get_keys(netuid);

        for k in &keys {
            let delegate_keys: Vec<U256> =
                (0..num_modules).map(|i| U256::from(i + num_modules + 1)).collect();
            for d in delegate_keys.iter() {
                SubspaceMod::add_balance_to_account(&d, stake_per_module + 1);
            }

            let pre_delegate_stake_from_vector = StakeFrom::<Test>::get(k);
            assert_eq!(pre_delegate_stake_from_vector.len(), 1); // +1 for the module itself, +1 for the delegate key on

            info!("KEY: {}", k);
            for (i, d) in delegate_keys.iter().enumerate() {
                info!("DELEGATE KEY: {d}");
                assert_ok!(SubspaceMod::add_stake(get_origin(*d), *k, stake_per_module));
                let stake_from_vector = StakeFrom::<Test>::get(k);
                assert_eq!(
                    stake_from_vector.len(),
                    pre_delegate_stake_from_vector.len() + i + 1
                );
            }
            let ownership_ratios: Vec<(U256, I64F64)> =
                SubnetConsensus::get_ownership_ratios(netuid, k);

            assert_eq!(ownership_ratios.len(), delegate_keys.len() + 1);
            info!("OWNERSHIP RATIOS: {ownership_ratios:?}");

            step_epoch(netuid);

            let stake_from_vector = StakeFrom::<Test>::get(k);
            let stake: u64 = Stake::<Test>::get(k);
            let sumed_stake: u64 = stake_from_vector.iter().fold(0, |acc, (_a, x)| acc + x);
            let total_stake: u64 = SubspaceMod::get_total_subnet_stake(netuid);

            info!("STAKE: {}", stake);
            info!("SUMED STAKE: {sumed_stake}");
            info!("TOTAL STAKE: {total_stake}");

            assert_eq!(stake, sumed_stake);
        }
    });
}

#[test]
fn test_min_stake_v2() {
    new_test_ext().execute_with(|| {
        let netuid: u16 = 0;
        let num_modules: u16 = 10;
        let min_stake: u64 = 10_000_000_000;
        // make sure that the results won´t get affected by burn
        zero_min_burn();

        register_n_modules(netuid, num_modules, min_stake);
        let keys = SubspaceMod::get_keys(netuid);

        update_params!(netuid => { min_stake: min_stake - 100 });

        assert_ok!(SubspaceMod::remove_stake(
            get_origin(keys[0]),
            keys[0],
            10_000_000_000
        ));
    });
}

#[test]
fn test_stake_zero() {
    new_test_ext().execute_with(|| {
        // Register the general subnet.
        let netuid: u16 = 0;
        let key = U256::from(0);
        let stake_amount: u64 = to_nano(1_000);

        // Make sure registration cost is not affected
        zero_min_burn();

        assert_ok!(register_module(netuid, key, stake_amount));

        // try to stake zero
        let key_two = U256::from(1);

        assert_noop!(
            SubspaceMod::do_add_stake(get_origin(key_two), key, 0),
            Error::<Test>::NotEnoughBalanceToStake
        );
    });
}

#[test]
fn test_unstake_zero() {
    new_test_ext().execute_with(|| {
        // Register the general subnet.
        let netuid: u16 = 0;
        let key = U256::from(0);
        let stake_amount: u64 = to_nano(1_000);

        // Make sure registration cost is not affected
        zero_min_burn();

        assert_ok!(register_module(netuid, key, stake_amount));

        // try to unstake zero
        let key_two = U256::from(1);

        assert_noop!(
            SubspaceMod::do_remove_stake(get_origin(key_two), key, 0),
            Error::<Test>::NotEnoughStakeToWithdraw
        );
    });
}

// Linear Step Consensus
// ======================

fn update_params(netuid: u16, tempo: u16, max_weights: u16, min_weights: u16) {
    Tempo::<Test>::insert(netuid, tempo);
    MaxAllowedWeights::<Test>::insert(netuid, max_weights);
    MinAllowedWeights::<Test>::insert(netuid, min_weights);
}

// FIXME:
// FIX global stake
// fn check_network_stats(netuid: u16) {
//     let emission_buffer: u64 = 1_000; // the numbers arent perfect but we want to make sure they
// fall within a range (10_000 / 2**64)     let threshold = SubnetStakeThreshold::<Test>::get();
//     let subnet_emission: u64 = SubnetEmissionMod::calculate_network_emission(netuid, threshold);
//     let incentives: Vec<u16> = Incentive::<Test>::get(netuid);
//     let dividends: Vec<u16> = Dividends::<Test>::get(netuid);
//     let emissions: Vec<u64> = Emission::<Test>::get(netuid);
//     let total_incentives: u16 = incentives.iter().sum();
//     let total_dividends: u16 = dividends.iter().sum();
//     let total_emissions: u64 = emissions.iter().sum();

//     info!("total_emissions: {total_emissions}");
//     info!("total_incentives: {total_incentives}");
//     info!("total_dividends: {total_dividends}");

//     info!("emission: {emissions:?}");
//     info!("incentives: {incentives:?}");
//     info!("dividends: {dividends:?}");

//     assert!(
//         total_emissions >= subnet_emission - emission_buffer
//             || total_emissions <= subnet_emission + emission_buffer
//     );
// }

#[test]
fn test_stale_weights() {
    new_test_ext().execute_with(|| {
        let netuid: u16 = 0;
        // make sure that the results won´t get affected by burn
        zero_min_burn();

        register_n_modules(0, 10, 1000);
        let _subnet_params = SubspaceMod::subnet_params(netuid);
        let _keys = SubspaceMod::get_keys(netuid);
        let _uids = SubspaceMod::get_uids(netuid);
    });
}

// FIXME:
// FIX GLOBAL STAKE
// #[test]
// fn test_no_weights() {
//     new_test_ext().execute_with(|| {
//         let netuid: u16 = 0;

//         // make sure that the results won´t get affected by burn
//         zero_min_burn();

//         register_n_modules(0, 10, 1000);
//         Tempo::<Test>::insert(netuid, 1);
//         let _keys = SubspaceMod::get_keys(netuid);
//         let _uids = SubspaceMod::get_uids(netuid);

//         let incentives: Vec<u16> = Incentive::<Test>::get(netuid);
//         let dividends: Vec<u16> = Dividends::<Test>::get(netuid);
//         let emissions: Vec<u64> = Emission::<Test>::get(netuid);
//         let _total_incentives: u16 = incentives.iter().sum();
//         let _total_dividends: u16 = dividends.iter().sum();
//         let _total_emissions: u64 = emissions.iter().sum();
//     });
// }

// #[test]
// fn test_dividends_same_stake() {
//     new_test_ext().execute_with(|| {
//         // CONSSTANTS
//         let netuid: u16 = 0;
//         let n: u16 = 10;
//         let _n_list: Vec<u16> = vec![10, 50, 100, 1000];
//         let _blocks_per_epoch_list: u64 = 1;
//         let stake_per_module: u64 = 10_000;

//         // make sure that the results won´t get affected by burn
//         zero_min_burn();

//         // SETUP NETWORK
//         register_n_modules(netuid, n, stake_per_module);
//         update_params(netuid, 1, n, 0);

//         let keys = SubspaceMod::get_keys(netuid);
//         let _uids = SubspaceMod::get_uids(netuid);

//         // do a list of ones for weights
//         let weight_uids: Vec<u16> = [2, 3].to_vec();
//         // do a list of ones for weights
//         let weight_values: Vec<u16> = [2, 1].to_vec();
//         set_weights(netuid, keys[0], weight_uids.clone(), weight_values.clone());
//         set_weights(netuid, keys[1], weight_uids.clone(), weight_values.clone());

//         let stakes_before: Vec<u64> = get_stakes(netuid);
//         step_epoch(netuid);
//         let incentives: Vec<u16> = Incentive::<Test>::get(netuid);
//         let dividends: Vec<u16> = Dividends::<Test>::get(netuid);
//         let emissions: Vec<u64> = Emission::<Test>::get(netuid);
//         let stakes: Vec<u64> = get_stakes(netuid);

//         // evaluate votees
//         assert!(incentives[2] > 0);
//         assert_eq!(dividends[2], dividends[3]);
//         let delta: u64 = 100;
//         assert!((incentives[2] as u64) > (weight_values[0] as u64 * incentives[3] as u64) -
// delta);         assert!((incentives[2] as u64) < (weight_values[0] as u64 * incentives[3] as u64)
// + delta);

//         assert!(emissions[2] > (weight_values[0] as u64 * emissions[3]) - delta);
//         assert!(emissions[2] < (weight_values[0] as u64 * emissions[3]) + delta);

//         // evaluate voters
//         assert!(
//             dividends[0] == dividends[1],
//             "dividends[0]: {} != dividends[1]: {}",
//             dividends[0],
//             dividends[1]
//         );
//         assert!(
//             dividends[0] == dividends[1],
//             "dividends[0]: {} != dividends[1]: {}",
//             dividends[0],
//             dividends[1]
//         );

//         assert_eq!(incentives[0], incentives[1]);
//         assert_eq!(dividends[2], dividends[3]);

//         info!("emissions: {emissions:?}");

//         for (uid, emission) in emissions.iter().enumerate() {
//             if emission == &0 {
//                 continue;
//             }
//             let stake: u64 = stakes[uid];
//             let stake_before: u64 = stakes_before[uid];
//             let stake_difference: u64 = stake - stake_before;
//             let expected_stake_difference: u64 = emissions[uid];
//             let error_delta: u64 = (emissions[uid] as f64 * 0.001) as u64;

//             assert!(
//                 stake_difference < expected_stake_difference + error_delta
//                     && stake_difference > expected_stake_difference - error_delta,
//                 "stake_difference: {} != expected_stake_difference: {}",
//                 stake_difference,
//                 expected_stake_difference
//             );
//         }

//         check_network_stats(netuid);
//     });
// }

// FIXME:
// FIX GLOBAL STAKE
// #[test]
// fn test_dividends_diff_stake() {
//     new_test_ext().execute_with(|| {
//         // CONSSTANTS
//         let netuid: u16 = 0;
//         let n: u16 = 10;
//         let _n_list: Vec<u16> = vec![10, 50, 100, 1000];
//         let _blocks_per_epoch_list: u64 = 1;
//         let stake_per_module: u64 = 10_000;
//         let tempo: u16 = 100;

//         // make sure that the results won´t get affected by burn
//         zero_min_burn();

//         // SETUP NETWORK
//         for i in 0..n {
//             let mut stake = stake_per_module;
//             if i == 0 {
//                 stake = 2 * stake_per_module
//             }
//             let key: U256 = U256::from(i);
//             assert_ok!(register_module(netuid, key, stake));
//         }
//         update_params(netuid, tempo, n, 0);

//         let keys = SubspaceMod::get_keys(netuid);
//         let _uids = SubspaceMod::get_uids(netuid);

//         // do a list of ones for weights
//         let weight_uids: Vec<u16> = [2, 3].to_vec();
//         // do a list of ones for weights
//         let weight_values: Vec<u16> = [1, 1].to_vec();
//         set_weights(netuid, keys[0], weight_uids.clone(), weight_values.clone());
//         set_weights(netuid, keys[1], weight_uids.clone(), weight_values.clone());

//         let stakes_before: Vec<u64> = get_stakes(netuid);
//         step_epoch(netuid);
//         let incentives: Vec<u16> = Incentive::<Test>::get(netuid);
//         let dividends: Vec<u16> = Dividends::<Test>::get(netuid);
//         let emissions: Vec<u64> = Emission::<Test>::get(netuid);
//         let stakes: Vec<u64> = get_stakes(netuid);

//         // evaluate votees
//         assert!(incentives[2] > 0);
//         assert_eq!(dividends[2], dividends[3]);
//         let delta: u64 = 100;
//         assert!((incentives[2] as u64) > (weight_values[0] as u64 * incentives[3] as u64) -
// delta);         assert!((incentives[2] as u64) < (weight_values[0] as u64 * incentives[3] as u64)
// + delta);

//         assert!(emissions[2] > (weight_values[0] as u64 * emissions[3]) - delta);
//         assert!(emissions[2] < (weight_values[0] as u64 * emissions[3]) + delta);

//         // evaluate voters
//         let delta: u64 = 100;
//         assert!((dividends[0] as u64) > (dividends[1] as u64 * 2) - delta);
//         assert!((dividends[0] as u64) < (dividends[1] as u64 * 2) + delta);

//         assert_eq!(incentives[0], incentives[1]);
//         assert_eq!(dividends[2], dividends[3]);

//         info!("emissions: {emissions:?}");

//         for (uid, emission) in emissions.iter().enumerate() {
//             if emission == &0 {
//                 continue;
//             }
//             let stake: u64 = stakes[uid];
//             let stake_before: u64 = stakes_before[uid];
//             let stake_difference: u64 = stake - stake_before;
//             let expected_stake_difference: u64 = emissions[uid];
//             let error_delta: u64 = (emissions[uid] as f64 * 0.001) as u64;

//             assert!(
//                 stake_difference < expected_stake_difference + error_delta
//                     && stake_difference > expected_stake_difference - error_delta,
//                 "stake_difference: {} != expected_stake_difference: {}",
//                 stake_difference,
//                 expected_stake_difference
//             );
//         }
//         check_network_stats(netuid);
//     });
// }

// FIXME
// FIXME: global stakes
// #[test]
// fn test_pruning() {
//     new_test_ext().execute_with(|| {
//         // CONSTANTS
//         let netuid: u16 = 0;
//         let n: u16 = 100;
//         let stake_per_module: u64 = 10_000;
//         let tempo: u16 = 100;

//         // make sure that the results won´t get affected by burn
//         zero_min_burn();
//         MaxRegistrationsPerBlock::<Test>::set(1000);

//         // SETUP NETWORK
//         register_n_modules(netuid, n, stake_per_module);
//         MaxAllowedModules::<Test>::put(n);
//         update_params(netuid, 1, n, 0);

//         let voter_idx = 0;
//         let keys = SubspaceMod::get_keys(netuid);
//         let _uids = SubspaceMod::get_uids(netuid);

//         // Create a list of UIDs excluding the voter_idx
//         let weight_uids: Vec<u16> = (0..n).filter(|&x| x != voter_idx as u16).collect();

//         // Create a list of ones for weights, excluding the voter_idx
//         let mut weight_values: Vec<u16> = weight_uids.iter().map(|_x| 1_u16).collect();

//         let prune_uid: u16 = weight_uids.last().cloned().unwrap_or(0);

//         if let Some(prune_idx) = weight_uids.iter().position(|&uid| uid == prune_uid) {
//             weight_values[prune_idx] = 0;
//         }

//         set_weights(
//             netuid,
//             keys[voter_idx as usize],
//             weight_uids.clone(),
//             weight_values.clone(),
//         );

//         step_block(tempo);

//         let lowest_priority_uid: u16 = SubspaceMod::get_lowest_uid(netuid, false);
//         assert!(lowest_priority_uid == prune_uid);

//         let new_key: U256 = U256::from(n + 1);

//         assert_ok!(register_module(netuid, new_key, stake_per_module));

//         let is_registered: bool = SubspaceMod::key_registered(netuid, &new_key);
//         assert!(is_registered);

//         assert!(
//             N::<Test>::get(netuid) == n,
//             "N::<Test>::get(netuid): {} != n: {}",
//             N::<Test>::get(netuid),
//             n
//         );

//         let is_prune_registered: bool =
//             SubspaceMod::key_registered(netuid, &keys[prune_uid as usize]);
//         assert!(!is_prune_registered);

//         check_network_stats(netuid);
//     });
// }

// FIXME:
// FIX GLOBAL STAKE
// #[test]
// fn test_lowest_priority_mechanism() {
//     new_test_ext().execute_with(|| {
//         // CONSSTANTS
//         let netuid: u16 = 0;
//         let n: u16 = 100;
//         let stake_per_module: u64 = 10_000;
//         let tempo: u16 = 100;

//         // make sure that the results won´t get affected by burn
//         zero_min_burn();
//         MaxRegistrationsPerBlock::<Test>::set(1000);

//         // SETUP NETWORK
//         register_n_modules(netuid, n, stake_per_module);

//         update_params(netuid, tempo, n, 0);

//         let keys = SubspaceMod::get_keys(netuid);
//         let voter_idx = 0;

//         // Create a list of UIDs excluding the voter_idx
//         let weight_uids: Vec<u16> = (0..n).filter(|&x| x != voter_idx).collect();

//         // Create a list of ones for weights, excluding the voter_idx
//         let mut weight_values: Vec<u16> = weight_uids.iter().map(|_x| 1_u16).collect();

//         let prune_uid: u16 = n - 1;

//         // Check if the prune_uid is still valid after excluding the voter_idx
//         if prune_uid != voter_idx {
//             // Find the index of prune_uid in the updated weight_uids vector
//             if let Some(prune_idx) = weight_uids.iter().position(|&uid| uid == prune_uid) {
//                 weight_values[prune_idx] = 0;
//             }
//         }

//         set_weights(
//             netuid,
//             keys[voter_idx as usize],
//             weight_uids.clone(),
//             weight_values.clone(),
//         );
//         step_block(tempo);
//         let incentives: Vec<u16> = Incentive::<Test>::get(netuid);
//         let dividends: Vec<u16> = Dividends::<Test>::get(netuid);
//         let emissions: Vec<u64> = Emission::<Test>::get(netuid);
//         let _stakes: Vec<u64> = get_stakes(netuid);

//         assert!(emissions[prune_uid as usize] == 0);
//         assert!(incentives[prune_uid as usize] == 0);
//         assert!(dividends[prune_uid as usize] == 0);

//         let lowest_priority_uid: u16 = SubspaceMod::get_lowest_uid(netuid, false);
//         info!("lowest_priority_uid: {lowest_priority_uid}");
//         info!("prune_uid: {prune_uid}");
//         info!("emissions: {emissions:?}");
//         info!("lowest_priority_uid: {lowest_priority_uid:?}");
//         info!("dividends: {dividends:?}");
//         info!("incentives: {incentives:?}");
//         assert!(lowest_priority_uid == prune_uid);
//         check_network_stats(netuid);
//     });
// }

#[test]
fn test_blocks_until_epoch() {
    new_test_ext().execute_with(|| {
        // Check tempo = 0 block = * netuid = *
        assert_eq!(SubnetEmissionMod::blocks_until_next_epoch(0, 0, 0), 1000);

        // Check tempo = 1 block = * netuid = *
        assert_eq!(SubnetEmissionMod::blocks_until_next_epoch(0, 1, 0), 0);
        assert_eq!(SubnetEmissionMod::blocks_until_next_epoch(1, 1, 0), 0);
        assert_eq!(SubnetEmissionMod::blocks_until_next_epoch(0, 1, 1), 0);
        assert_eq!(SubnetEmissionMod::blocks_until_next_epoch(1, 2, 1), 0);
        assert_eq!(SubnetEmissionMod::blocks_until_next_epoch(0, 4, 3), 3);
        assert_eq!(SubnetEmissionMod::blocks_until_next_epoch(10, 5, 2), 2);
        // Check general case.
        for netuid in 0..30_u16 {
            for block in 0..30_u64 {
                for tempo in 1..30_u16 {
                    assert_eq!(
                        SubnetEmissionMod::blocks_until_next_epoch(netuid, tempo, block),
                        (block + netuid as u64) % (tempo as u64)
                    );
                }
            }
        }
    });
}

#[test]
fn test_incentives() {
    new_test_ext().execute_with(|| {
        // CONSSTANTS
        let netuid: u16 = 0;
        let n: u16 = 10;
        let _n_list: Vec<u16> = vec![10, 50, 100, 1000];
        let _blocks_per_epoch_list: u64 = 1;
        let stake_per_module: u64 = 10_000;

        // make sure that the results won´t get affected by burn
        zero_min_burn();

        // SETUP NETWORK
        register_n_modules(netuid, n, stake_per_module);
        let mut params = SubspaceMod::subnet_params(netuid);
        params.min_allowed_weights = 0;
        params.max_allowed_weights = n;
        params.tempo = 100;

        let keys = SubspaceMod::get_keys(netuid);
        let _uids = SubspaceMod::get_uids(netuid);

        // do a list of ones for weights
        let weight_uids: Vec<u16> = [1, 2].to_vec();
        // do a list of ones for weights
        let weight_values: Vec<u16> = [1, 1].to_vec();

        set_weights(netuid, keys[0], weight_uids.clone(), weight_values.clone());
        step_block(params.tempo);

        let incentives: Vec<u16> = Incentive::<Test>::get(netuid);
        let emissions: Vec<u64> = Emission::<Test>::get(netuid);

        // evaluate votees
        assert!(incentives[1] > 0);
        assert!(incentives[1] == incentives[2]);
        assert!(emissions[1] == emissions[2]);

        // do a list of ones for weights
        let weight_values: Vec<u16> = [1, 2].to_vec();

        set_weights(netuid, keys[0], weight_uids.clone(), weight_values.clone());
        set_weights(netuid, keys[9], weight_uids.clone(), weight_values.clone());

        step_block(params.tempo);

        let incentives: Vec<u16> = Incentive::<Test>::get(netuid);
        let emissions: Vec<u64> = Emission::<Test>::get(netuid);

        // evaluate votees
        let delta: u64 = 100 * params.tempo as u64;
        assert!(incentives[1] > 0);

        assert!(
            emissions[2] > 2 * emissions[1] - delta && emissions[2] < 2 * emissions[1] + delta,
            "emissions[1]: {} != emissions[2]: {}",
            emissions[1],
            emissions[2]
        );
    });
}

#[test]
fn test_trust() {
    new_test_ext().execute_with(|| {
        // CONSSTANTS
        let netuid: u16 = 0;
        let n: u16 = 10;
        let _n_list: Vec<u16> = vec![10, 50, 100, 1000];
        let _blocks_per_epoch_list: u64 = 1;
        let stake_per_module: u64 = 10_000;
        // make sure that the results won´t get affected by burn
        zero_min_burn();

        // SETUP NETWORK
        register_n_modules(netuid, n, stake_per_module);
        let mut params = SubspaceMod::subnet_params(netuid);
        params.min_allowed_weights = 1;
        params.max_allowed_weights = n;
        params.tempo = 100;
        params.trust_ratio = 100;

        update_params!(netuid => params.clone());

        let keys = SubspaceMod::get_keys(netuid);
        let _uids = SubspaceMod::get_uids(netuid);

        // do a list of ones for weights
        let weight_uids: Vec<u16> = [2].to_vec();
        let weight_values: Vec<u16> = [1].to_vec();

        set_weights(netuid, keys[8], weight_uids.clone(), weight_values.clone());
        // do a list of ones for weights
        let weight_uids: Vec<u16> = [1, 2].to_vec();
        let weight_values: Vec<u16> = [1, 1].to_vec();
        set_weights(netuid, keys[9], weight_uids.clone(), weight_values.clone());
        step_block(params.tempo);

        let trust: Vec<u16> = Trust::<Test>::get(netuid);
        let emission: Vec<u64> = Emission::<Test>::get(netuid);

        // evaluate votees
        info!("trust: {:?}", trust);
        assert!(trust[1] as u32 > 0);
        assert!(trust[2] as u32 > 2 * (trust[1] as u32) - 10);
        // evaluate votees
        info!("trust: {emission:?}");
        assert!(emission[1] > 0);
        assert!(emission[2] > 2 * (emission[1]) - 1000);

        // assert!(trust[2] as u32 < 2*(trust[1] as u32)   );
    });
}

// FIXME:
// Fix this one
// GLOBAL STAKE
// #[test]
// fn test_founder_share() {
//     new_test_ext().execute_with(|| {
//         let netuid = 0;
//         let n = 20;
//         let initial_stake: u64 = 1000;
//         let keys: Vec<U256> = (0..n).map(U256::from).collect();
//         let stakes: Vec<u64> = (0..n).map(|_x| initial_stake * 1_000_000_000).collect();

//         let founder_key = keys[0];
//         MaxRegistrationsPerBlock::<Test>::set(1000);
//         for i in 0..n {
//             assert_ok!(register_module(netuid, keys[i], stakes[i]));
//             let stake_from_vector = Stake::<Test>::get(&keys[i]);
//             info!("{:?}", stake_from_vector);
//         }
//         update_params!(netuid => { founder_share: 12 });
//         let founder_share = FounderShare::<Test>::get(netuid);
//         let founder_ratio: f64 = founder_share as f64 / 100.0;

//         let subnet_params = SubspaceMod::subnet_params(netuid);

//         let founder_stake_before = Stake::<Test>::get(founder_key);
//         info!("founder_stake_before: {founder_stake_before:?}");
//         // vote to avoid key[0] as we want to see the key[0] burn
//         step_epoch(netuid);
//         let threshold = SubnetStakeThreshold::<Test>::get();
//         let total_emission =
//             SubnetEmissionMod::calculate_network_emission(netuid, threshold) *
// subnet_params.tempo as u64;         let expected_founder_share = (total_emission as f64 *
// founder_ratio) as u64;         let expected_emission = total_emission - expected_founder_share;
//         let emissions = Emission::<Test>::get(netuid);
//         let dividends = Dividends::<Test>::get(netuid);
//         let incentives = Incentive::<Test>::get(netuid);
//         let total_dividends: u64 = dividends.iter().sum::<u16>() as u64;
//         let total_incentives: u64 = incentives.iter().sum::<u16>() as u64;

//         let founder_dividend_emission = ((dividends[0] as f64 / total_dividends as f64)
//             * (expected_emission / 2) as f64) as u64;
//         let founder_incentive_emission = ((incentives[0] as f64 / total_incentives as f64)
//             * (expected_emission / 2) as f64) as u64;
//         let founder_emission = founder_incentive_emission + founder_dividend_emission;

//         let calcualted_total_emission = emissions.iter().sum::<u64>();

//         let key_stake = Stake::<Test>::get(founder_key);
//         let founder_total_stake = founder_stake_before + founder_emission;
//         assert_eq!(
//             key_stake - (key_stake % 1000),
//             founder_total_stake - (founder_total_stake % 1000)
//         );
//         assert_eq!(
//             SubspaceMod::get_balance(&DaoTreasuryAddress::<Test>::get()),
//             expected_founder_share - 1 /* Account for rounding errors */
//         );

//         assert_eq!(
//             expected_emission - (expected_emission % 100000),
//             calcualted_total_emission - (calcualted_total_emission % 100000)
//         );
//     });
// }

#[test]
fn test_dynamic_burn() {
    new_test_ext().execute_with(|| {
        let netuid = 0;
        let initial_stake: u64 = 1000;

        // make sure that the results won´t get affected by burn
        zero_min_burn();

        // Create the subnet
        let subnet_key = U256::from(2050);
        assert_ok!(register_module(netuid, subnet_key, initial_stake));
        // Using the default GlobalParameters:
        // - registration target interval = 2 * tempo (200 blocks)
        // - registration target for interval = registration_target_interval / 2
        // - adjustment alpha = 0
        // - min_burn = 2 $COMAI
        // - max_burn = 250 $COMAI
        let burn_config = BurnConfiguration {
            min_burn: to_nano(2),
            max_burn: to_nano(250),
            adjustment_alpha: 0,
            adjustment_interval: 200,
            expected_registrations: 100,
            ..BurnConfiguration::<Test>::default()
        };
        assert_ok!(burn_config.apply());

        let BurnConfiguration { min_burn, .. } = BurnConfig::<Test>::get();

        // update the burn to the minimum
        step_block(200);

        assert!(
            Burn::<Test>::get(netuid) == min_burn,
            "current burn: {:?}",
            Burn::<Test>::get(netuid)
        );

        // Register the first 1000 modules, this is 10x the registration target
        let registrations_per_block = 5;
        let n: usize = 1000;
        let stakes: Vec<u64> = (0..n).map(|_| initial_stake * 1_000_000_000).collect();
        for (i, stake) in stakes.iter().enumerate() {
            let key = U256::from(i);
            assert_ok!(register_module(netuid, key, *stake));
            if (i + 1) % registrations_per_block == 0 {
                step_block(1);
            }
        }

        // Burn is now at 11 instead of 2
        assert!(
            Burn::<Test>::get(netuid) == to_nano(11),
            "current burn {:?}",
            Burn::<Test>::get(netuid)
        );

        MaxRegistrationsPerBlock::<Test>::set(1000);
        // Register only 50 of the target
        let amount: usize = 50;
        for (i, &stake) in stakes.iter().enumerate().take(amount) {
            let key = U256::from(n + i);
            assert_ok!(register_module(netuid, key, stake));
        }

        step_block(200);

        // Make sure the burn correctly decreased based on demand
        assert!(
            Burn::<Test>::get(netuid) == 8250000000,
            "current burn: {:?}",
            Burn::<Test>::get(netuid)
        );
    });
}

// Deprecated Global Burn
// #[test]
// fn test_dao_treasury_distribution_for_subnet_owners() {
//     new_test_ext().execute_with(|| {
//     });

// YUMA consensus
//==============

mod utils {
    use pallet_subspace::{Consensus, Dividends, Emission, Incentive, Rank, Trust};

    use crate::subspace::Test;

    pub fn get_rank_for_uid(netuid: u16, uid: u16) -> u16 {
        Rank::<Test>::get(netuid).get(uid as usize).copied().unwrap_or_default()
    }

    pub fn get_trust_for_uid(netuid: u16, uid: u16) -> u16 {
        Trust::<Test>::get(netuid).get(uid as usize).copied().unwrap_or_default()
    }

    pub fn get_consensus_for_uid(netuid: u16, uid: u16) -> u16 {
        Consensus::<Test>::get(netuid).get(uid as usize).copied().unwrap_or_default()
    }

    pub fn get_incentive_for_uid(netuid: u16, uid: u16) -> u16 {
        Incentive::<Test>::get(netuid).get(uid as usize).copied().unwrap_or_default()
    }

    pub fn get_dividends_for_uid(netuid: u16, uid: u16) -> u16 {
        Dividends::<Test>::get(netuid).get(uid as usize).copied().unwrap_or_default()
    }

    pub fn get_emission_for_uid(netuid: u16, uid: u16) -> u64 {
        Emission::<Test>::get(netuid).get(uid as usize).copied().unwrap_or_default()
    }
}

const ONE: u64 = to_nano(1);

// We are off by one,
// due to inactive / active calulation on yuma, which is 100% correct.
#[test]
#[ignore = "global stake ?"]
fn test_1_graph() {
    new_test_ext().execute_with(|| {
        UnitEmission::<Test>::set(23148148148);
        zero_min_burn();

        // Register general subnet
        assert_ok!(register_module(0, 10.into(), 1));

        log::info!("test_1_graph:");
        let netuid: u16 = 1;
        let key = U256::from(0);
        let uid: u16 = 0;
        let stake_amount: u64 = to_nano(100);

        assert_ok!(register_module(netuid, key, stake_amount));
        update_params!(netuid => {
            max_allowed_uids: 2
        });

        assert_ok!(register_module(netuid, key + 1, 1));
        assert_eq!(N::<Test>::get(netuid), 2);

        run_to_block(1); // run to next block to ensure weights are set on nodes after their registration block

        assert_ok!(SubspaceMod::set_weights(
            RuntimeOrigin::signed(U256::from(1)),
            netuid,
            vec![uid],
            vec![u16::MAX],
        ));

        let emissions = YumaEpoch::<Test>::new(netuid, ONE).run();
        let offset = 1;

        assert_eq!(
            emissions.unwrap(),
            [(ModuleKey(key), [(AccountKey(key), ONE - offset)].into())].into()
        );

        let new_stake_amount = stake_amount + ONE;

        assert_eq!(
            SubspaceMod::get_total_stake_to(&key),
            new_stake_amount - offset
        );
        assert_eq!(utils::get_rank_for_uid(netuid, uid), 0);
        assert_eq!(utils::get_trust_for_uid(netuid, uid), 0);
        assert_eq!(utils::get_consensus_for_uid(netuid, uid), 0);
        assert_eq!(utils::get_incentive_for_uid(netuid, uid), 0);
        assert_eq!(utils::get_dividends_for_uid(netuid, uid), 0);
        assert_eq!(utils::get_emission_for_uid(netuid, uid), ONE - offset);
    });
}

#[test]
fn test_10_graph() {
    /// Function for adding a nodes to the graph.
    fn add_node(netuid: u16, key: U256, uid: u16, stake_amount: u64) {
        log::info!(
            "+Add net:{:?} hotkey:{:?} uid:{:?} stake_amount: {:?} subn: {:?}",
            netuid,
            key,
            uid,
            stake_amount,
            N::<Test>::get(netuid),
        );

        assert_ok!(register_module(netuid, key, stake_amount));
        assert_eq!(N::<Test>::get(netuid) - 1, uid);
    }

    new_test_ext().execute_with(|| {
        UnitEmission::<Test>::put(23148148148);
        zero_min_burn();
        FloorFounderShare::<Test>::put(0);
        MaxRegistrationsPerBlock::<Test>::set(1000);
        // Register general subnet
        assert_ok!(register_module(0, 10_000.into(), 1));

        log::info!("test_10_graph");

        // Build the graph with 10 items
        // each with 1 stake and self weights.
        let n: usize = 10;
        let netuid: u16 = 1;
        let stake_amount_per_node = ONE;

        for i in 0..n {
            add_node(netuid, U256::from(i), i as u16, stake_amount_per_node)
        }

        update_params!(netuid => {
            max_allowed_uids: n as u16 + 1
        });

        assert_ok!(register_module(netuid, U256::from(n + 1), 1));
        assert_eq!(N::<Test>::get(netuid), 11);

        run_to_block(1); // run to next block to ensure weights are set on nodes after their registration block

        for i in 0..n {
            assert_ok!(SubspaceMod::set_weights(
                RuntimeOrigin::signed(U256::from(n + 1)),
                netuid,
                vec![i as u16],
                vec![u16::MAX],
            ));
        }

        let emissions = YumaEpoch::<Test>::new(netuid, ONE).run();
        let mut expected: EmissionMap<Test> = BTreeMap::new();

        // Check return values.
        let emission_per_node = ONE / n as u64;
        for i in 0..n as u16 {
            assert_eq!(
                from_nano(Stake::<Test>::get(&(U256::from(i)))),
                from_nano(to_nano(1) + emission_per_node)
            );

            assert_eq!(utils::get_rank_for_uid(netuid, i), 0);
            assert_eq!(utils::get_trust_for_uid(netuid, i), 0);
            assert_eq!(utils::get_consensus_for_uid(netuid, i), 0);
            assert_eq!(utils::get_incentive_for_uid(netuid, i), 0);
            assert_eq!(utils::get_dividends_for_uid(netuid, i), 0);
            assert_eq!(utils::get_emission_for_uid(netuid, i), 99999999);

            expected
                .entry(ModuleKey(i.into()))
                .or_default()
                .insert(AccountKey(i.into()), 99999999);
        }

        assert_eq!(emissions.unwrap(), expected);
    });
}
// Testing weight expiration, on subnets running yuma
#[test]
fn yuma_weights_older_than_max_age_are_discarded() {
    new_test_ext().execute_with(|| {
        const MAX_WEIGHT_AGE: u64 = 300;
        const SUBNET_TEMPO: u16 = 100;
        // Register the general subnet.
        let netuid: u16 = 0;
        let key = U256::from(0);
        let stake_amount: u64 = to_nano(1_000);

        assert_ok!(register_module(netuid, key, stake_amount));

        // Register the yuma subnet.
        let yuma_netuid: u16 = 1;
        let yuma_validator_key = U256::from(1);
        let yuma_miner_key = U256::from(2);
        let yuma_vali_amount: u64 = to_nano(10_000);
        let yuma_miner_amount = to_nano(1_000);

        // This will act as an validator.
        assert_ok!(register_module(
            yuma_netuid,
            yuma_validator_key,
            yuma_vali_amount
        ));
        // This will act as an miner.
        assert_ok!(register_module(
            yuma_netuid,
            yuma_miner_key,
            yuma_miner_amount
        ));

        step_block(1);

        // Set the max weight age to 300 blocks
        update_params!(yuma_netuid => {
            tempo: SUBNET_TEMPO,
            max_weight_age: MAX_WEIGHT_AGE
        });

        let miner_uid = SubspaceMod::get_uid_for_key(yuma_netuid, &yuma_miner_key);
        let validator_uid = SubspaceMod::get_uid_for_key(yuma_netuid, &yuma_validator_key);
        let uid = [miner_uid].to_vec();
        let weight = [1].to_vec();

        // set the weights
        assert_ok!(SubspaceMod::do_set_weights(
            get_origin(yuma_validator_key),
            yuma_netuid,
            uid,
            weight
        ));

        step_block(100);

        // Make sure we have incentive and dividends
        let miner_incentive = SubspaceMod::get_incentive_for_uid(yuma_netuid, miner_uid);
        let miner_dividends = SubspaceMod::get_dividends_for_uid(yuma_netuid, miner_uid);
        let validator_incentive = SubspaceMod::get_incentive_for_uid(yuma_netuid, validator_uid);
        let validator_dividends = SubspaceMod::get_dividends_for_uid(yuma_netuid, validator_uid);

        assert!(miner_incentive > 0);
        assert_eq!(miner_dividends, 0);
        assert!(validator_dividends > 0);
        assert_eq!(validator_incentive, 0);

        // now go pass the max weight age
        step_block(MAX_WEIGHT_AGE as u16);

        // Make sure we have no incentive and dividends
        let miner_incentive = SubspaceMod::get_incentive_for_uid(yuma_netuid, miner_uid);
        let miner_dividends = SubspaceMod::get_dividends_for_uid(yuma_netuid, miner_uid);
        let validator_incentive = SubspaceMod::get_incentive_for_uid(yuma_netuid, validator_uid);
        let validator_dividends = SubspaceMod::get_dividends_for_uid(yuma_netuid, validator_uid);

        assert_eq!(miner_incentive, 0);
        assert_eq!(miner_dividends, 0);
        assert_eq!(validator_dividends, 0);
        assert_eq!(validator_incentive, 0);

        // But make sure there are emissions

        let subnet_emission_sum = SubnetEmission::<Test>::get(yuma_netuid);
        assert!(subnet_emission_sum > 0);
    });
}

// Bad actor will try to move stake quickly from one subnet to another,
// in hopes of increasing their emissions.
// Logic is getting above the subnet_stake threshold with a faster tempo
// (this is not possible due to emissions_to_drain calculated at evry block, making such exploits
// impossible)
#[test]
#[ignore = "global stake?"]
fn test_emission_exploit() {
    new_test_ext().execute_with(|| {
        const SUBNET_TEMPO: u16 = 25;
        // Register the general subnet.
        let netuid: u16 = 0;
        let key = U256::from(0);
        let stake_amount: u64 = to_nano(1_000);

        // Make sure registration cost is not affected
        zero_min_burn();

        assert_ok!(register_module(netuid, key, stake_amount));

        // Register the yuma subnet.
        let yuma_netuid: u16 = 1;
        let yuma_badactor_key = U256::from(1);
        let yuma_badactor_amount: u64 = to_nano(10_000);

        assert_ok!(register_module(
            yuma_netuid,
            yuma_badactor_key,
            yuma_badactor_amount
        ));
        update_params!(netuid => { tempo: SUBNET_TEMPO });

        // step first 40 blocks from the registration
        step_block(40);

        let stake_accumulated = Stake::<Test>::get(&yuma_badactor_key);
        // User will now unstake and register another subnet.
        assert_ok!(SubspaceMod::do_remove_stake(
            get_origin(yuma_badactor_key),
            yuma_badactor_key,
            stake_accumulated - 1
        ));

        // simulate real conditions by stepping  block
        step_block(2); // 42 blocks passed since the registration

        let new_netuid = 2;
        // register the new subnet
        let mut network: Vec<u8> = "test".as_bytes().to_vec();
        network.extend(new_netuid.to_string().as_bytes().to_vec());
        let mut name: Vec<u8> = "module".as_bytes().to_vec();
        name.extend(key.to_string().as_bytes().to_vec());
        let address: Vec<u8> = "0.0.0.0:30333".as_bytes().to_vec();
        let origin = get_origin(yuma_badactor_key);
        assert_ok!(SubspaceMod::register(
            origin,
            network,
            name,
            address,
            yuma_badactor_amount - 1,
            yuma_badactor_key,
            None
        ));

        // set the tempo
        update_params!(netuid => { tempo: SUBNET_TEMPO });

        // now 100 blocks went by since the registration, 1 + 40 + 58 = 100
        step_block(58);

        // remove the stake again
        let stake_accumulated_two = Stake::<Test>::get(&yuma_badactor_key);
        assert_ok!(SubspaceMod::do_remove_stake(
            get_origin(yuma_badactor_key),
            yuma_badactor_key,
            stake_accumulated_two - 2
        ));

        let badactor_balance_after = SubspaceMod::get_balance(&yuma_badactor_key);

        let new_netuid = 3;
        // Now an honest actor will come, the goal is for him to accumulate more
        let honest_actor_key = U256::from(3);
        assert_ok!(register_module(
            new_netuid,
            honest_actor_key,
            yuma_badactor_amount
        ));
        // we will set a slower tempo, standard 100
        update_params!(new_netuid => { tempo: 100 });
        step_block(101);

        // get the stake of honest actor
        let hones_stake = Stake::<Test>::get(&honest_actor_key);
        dbg!(hones_stake);
        dbg!(badactor_balance_after);

        assert!(hones_stake > badactor_balance_after);
    });
}

#[test]
#[ignore = "global stake?"]
fn test_tempo_compound() {
    new_test_ext().execute_with(|| {
        const QUICK_TEMPO: u16 = 25;
        const SLOW_TEMPO: u16 = 1000;
        // Register the general subnet.
        let netuid: u16 = 0;
        let key = U256::from(0);
        let stake_amount: u64 = to_nano(1_000);

        // Make sure registration cost is not affected
        zero_min_burn();

        assert_ok!(register_module(netuid, key, stake_amount));

        // Register the yuma subnets, the important part of the tests starts here:
        // FAST
        let s_netuid: u16 = 1;
        let s_key = U256::from(1);
        let s_amount: u64 = to_nano(10_000);

        assert_ok!(register_module(s_netuid, s_key, s_amount));
        update_params!(s_netuid => { tempo: SLOW_TEMPO });

        // SLOW
        let f_netuid = 2;
        // Now an honest actor will come, the goal is for him to accumulate more
        let f_key = U256::from(3);
        assert_ok!(register_module(f_netuid, f_key, s_amount));
        // we will set a slower tempo
        update_params!(f_netuid => { tempo: QUICK_TEMPO });

        // we will now step, SLOW_TEMPO -> 1000 blocks
        step_block(SLOW_TEMPO);

        let fast = Stake::<Test>::get(&f_key);
        let slow = Stake::<Test>::get(&s_key);

        // faster tempo should have quicker compound rate
        assert!(fast > slow);
    });
}

// Subnets
//========

#[test]
#[ignore = "global stake?"]
fn test_add_subnets() {
    new_test_ext().execute_with(|| {
        let _tempo: u16 = 13;
        let stake_per_module: u64 = 1_000_000_000;
        let max_allowed_subnets: u16 = MaxAllowedSubnets::<Test>::get();
        let mut expected_subnets = 0;
        let n = 20;
        let num_subnets: u16 = n;

        // make sure that the results won´t get affected by burn
        zero_min_burn();
        MaxRegistrationsPerBlock::<Test>::set(1000);

        for i in 0..num_subnets {
            assert_ok!(register_module(i, U256::from(i), stake_per_module));
            for j in 0..n {
                if j != i {
                    let n = N::<Test>::get(i);
                    info!("registering module i:{} j:{} n:{}", i, j, n);
                    assert_ok!(register_module(i, U256::from(j), stake_per_module));
                }
            }
            expected_subnets += 1;
            if expected_subnets > max_allowed_subnets {
                expected_subnets = max_allowed_subnets;
            } else {
                assert_eq!(N::<Test>::get(i), n);
            }
            assert_eq!(
                TotalSubnets::<Test>::get(),
                expected_subnets,
                "number of subnets is not equal to expected subnets"
            );
        }

        for netuid in 0..num_subnets {
            let total_stake = SubspaceMod::get_total_subnet_stake(netuid);
            let total_balance = get_total_subnet_balance(netuid);
            let total_tokens_before = total_stake + total_balance;

            let keys = SubspaceMod::get_keys(netuid);

            info!("total stake {total_stake}");
            info!("total balance {total_balance}");
            info!("total tokens before {total_tokens_before}");

            assert_eq!(keys.len() as u16, n);
            assert!(check_subnet_storage(netuid));
            SubspaceMod::remove_subnet(netuid);
            assert_eq!(N::<Test>::get(netuid), 0);
            assert!(check_subnet_storage(netuid));

            let total_tokens_after: u64 = keys.iter().map(SubspaceMod::get_balance_u64).sum();
            info!("total tokens after {}", total_tokens_after);

            assert_eq!(total_tokens_after, total_tokens_before);
            expected_subnets = expected_subnets.saturating_sub(1);
            assert_eq!(
                TotalSubnets::<Test>::get(),
                expected_subnets,
                "number of subnets is not equal to expected subnets"
            );
        }
    });
}

// FIXME:
// GLOBAL STAKE
// #[test]
// fn test_emission_ratio() {
//     new_test_ext().execute_with(|| {
//         let netuids: Vec<u16> = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9].to_vec();
//         let stake_per_module: u64 = 1_000_000_000;
//         let mut emissions_per_subnet: Vec<u64> = Vec::new();
//         let max_delta: f64 = 1.0;
//         let _n: u16 = 10;

//         // make sure that the results won´t get affected by burn
//         zero_min_burn();

//         for i in 0..netuids.len() {
//             let _key = U256::from(netuids[i]);
//             let netuid = netuids[i];
//             register_n_modules(netuid, 1, stake_per_module);
//             let threshold = SubnetStakeThreshold::<Test>::get();
//             let subnet_emission: u64 =
//                 SubnetEmissionMod::calculate_network_emission(netuid, threshold);
//             emissions_per_subnet.push(subnet_emission);
//             let _expected_emission_factor: f64 = 1.0 / (netuids.len() as f64);
//             let emission_per_block = SubnetEmissionMod::get_total_emission_per_block();
//             let expected_emission: u64 = emission_per_block / (i as u64 + 1);

//             let block = block_number();
//             // magnitude of difference between expected and actual emission
//             let delta = if subnet_emission > expected_emission {
//                 subnet_emission - expected_emission
//             } else {
//                 expected_emission - subnet_emission
//             } as f64;

//             assert!(
//                 delta <= max_delta,
//                 "emission {} is too far from expected emission {} ",
//                 subnet_emission,
//                 expected_emission
//             );
//             assert!(block == 0, "block {} is not 0", block);
//             info!("block {} subnet_emission {} ", block, subnet_emission);
//         }
//     });
// }

#[test]
fn test_set_max_allowed_uids_growing() {
    new_test_ext().execute_with(|| {
        let netuid: u16 = 0;
        let stake: u64 = 1_000_000_000;
        let mut max_uids: u16 = 100;
        let extra_uids: u16 = 10;
        let rounds = 10;

        // make sure that the results won´t get affected by burn
        zero_min_burn();

        assert_ok!(register_module(netuid, U256::from(0), stake));
        MaxRegistrationsPerBlock::<Test>::set(max_uids + extra_uids * rounds);
        for i in 1..max_uids {
            assert_ok!(register_module(netuid, U256::from(i), stake));
            assert_eq!(N::<Test>::get(netuid), i + 1);
        }
        let mut n: u16 = N::<Test>::get(netuid);
        let old_n: u16 = n;
        let mut _uids: Vec<u16>;
        assert_eq!(N::<Test>::get(netuid), max_uids);
        for r in 1..rounds {
            // set max allowed uids to max_uids + extra_uids
            update_params!(netuid => {
                max_allowed_uids: max_uids + extra_uids * (r - 1)
            });
            max_uids = MaxAllowedUids::<Test>::get(netuid);
            let new_n = old_n + extra_uids * (r - 1);
            // print the pruned uids
            for uid in old_n + extra_uids * (r - 1)..old_n + extra_uids * r {
                assert_ok!(register_module(netuid, U256::from(uid), stake));
            }

            // set max allowed uids to max_uids

            n = N::<Test>::get(netuid);
            assert_eq!(n, new_n);

            let uids = SubspaceMod::get_uids(netuid);
            assert_eq!(uids.len() as u16, n);

            let keys = SubspaceMod::get_keys(netuid);
            assert_eq!(keys.len() as u16, n);

            let names = SubspaceMod::get_names(netuid);
            assert_eq!(names.len() as u16, n);

            let addresses = SubspaceMod::get_addresses(netuid);
            assert_eq!(addresses.len() as u16, n);
        }
    });
}

#[test]
fn test_set_max_allowed_uids_shrinking() {
    new_test_ext().execute_with(|| {
        let netuid: u16 = 0;
        let stake: u64 = 1_000_000_000;
        let max_uids: u16 = 100;
        let extra_uids: u16 = 20;

        // make sure that the results won´t get affected by burn
        zero_min_burn();

        let mut n = N::<Test>::get(netuid);
        info!("registering module {}", n);
        assert_ok!(register_module(netuid, U256::from(0), stake));
        update_params!(netuid => {
            max_allowed_uids: max_uids + extra_uids
        });
        MaxRegistrationsPerBlock::<Test>::set(max_uids + extra_uids);

        for i in 1..(max_uids + extra_uids) {
            let result = register_module(netuid, U256::from(i), stake);
            result.unwrap();
            n = N::<Test>::get(netuid);
        }

        assert_eq!(n, max_uids + extra_uids);

        let keys = SubspaceMod::get_keys(netuid);

        let mut total_stake: u64 = SubspaceMod::get_total_subnet_stake(netuid);
        let mut expected_stake: u64 = n as u64 * stake;

        info!("total stake {total_stake}");
        info!("expected stake {expected_stake}");
        assert_eq!(total_stake, expected_stake);

        let mut params = SubspaceMod::subnet_params(netuid).clone();
        params.max_allowed_uids = max_uids;
        params.name = "test2".as_bytes().to_vec().try_into().unwrap();
        let result = SubspaceMod::update_subnet(
            get_origin(keys[0]),
            netuid,
            params.founder,
            params.founder_share,
            params.immunity_period,
            params.incentive_ratio,
            params.max_allowed_uids,
            params.max_allowed_weights,
            params.min_allowed_weights,
            params.max_weight_age,
            params.min_stake,
            params.name.clone(),
            params.tempo,
            params.trust_ratio,
            params.maximum_set_weight_calls_per_epoch,
            params.vote_mode,
            params.bonds_ma,
        );
        let global_params = SubspaceMod::global_params();
        info!("global params {:?}", global_params);
        info!("subnet params {:?}", SubspaceMod::subnet_params(netuid));
        assert_ok!(result);
        let params = SubspaceMod::subnet_params(netuid);
        let n = N::<Test>::get(netuid);
        assert_eq!(
            params.max_allowed_uids, max_uids,
            "max allowed uids is not equal to expected max allowed uids"
        );
        assert_eq!(
            params.max_allowed_uids, n,
            "min allowed weights is not equal to expected min allowed weights"
        );

        expected_stake = (max_uids) as u64 * stake;
        let _subnet_stake = SubspaceMod::get_total_subnet_stake(netuid);
        total_stake = SubspaceMod::total_stake();

        assert_eq!(total_stake, expected_stake);
    });
}

#[test]
fn test_set_max_allowed_modules() {
    new_test_ext().execute_with(|| {
        let netuid: u16 = 0;
        let stake: u64 = 1_000_000_000;
        let _max_uids: u16 = 2000;
        let _extra_uids: u16 = 20;
        let max_allowed_modules: u16 = 100;

        // make sure that the results won´t get affected by burn
        zero_min_burn();
        MaxRegistrationsPerBlock::<Test>::set(1000);
        MaxAllowedModules::<Test>::put(max_allowed_modules);
        // set max_total modules

        for i in 1..(2 * max_allowed_modules) {
            assert_ok!(register_module(netuid, U256::from(i), stake));
            let n = N::<Test>::get(netuid);
            assert!(
                n <= max_allowed_modules,
                "subnet_n {:?} is not less than max_allowed_modules {:?}",
                n,
                max_allowed_modules
            );
        }
    })
}

#[test]
fn test_deregister_subnet_when_overflows_max_allowed_subnets() {
    new_test_ext().execute_with(|| {
        let extra = 1;
        let mut params = SubspaceMod::global_params();
        params.max_allowed_subnets = 3;
        SubspaceMod::set_global_params(params.clone());
        // make sure that the results won´t get affected by burn
        zero_min_burn();

        assert_eq!(params.max_allowed_subnets, 3);

        let stakes: Vec<u64> = vec![
            2_000_000_000,
            6_000_000_000,
            3_000_000_000,
            4_000_000_000,
            9_000_000_000,
        ];

        for netuid in 0..params.max_allowed_subnets + extra {
            let stake: u64 = stakes[netuid as usize];
            assert_ok!(register_module(netuid, U256::from(netuid), stake));
        }

        assert_eq!(SubspaceMod::get_total_subnet_stake(1), stakes[1]);
        assert_eq!(SubspaceMod::get_total_subnet_stake(2), stakes[2]);
        assert_eq!(SubspaceMod::get_total_subnet_stake(0), stakes[3]);
        assert_eq!(TotalSubnets::<Test>::get(), 3);
    });
}

// FIXME:
// GLOBAL STAKE
// #[test]
// fn test_emission_distribution_novote() {
//     // test if subnet emissions are distributed correctly, even without voting
//     new_test_ext().execute_with(|| {
//         let netuid_general: u16 = 0; // hold 50% of the networks stake
//         let stake_general: u64 = to_nano(500_000);

//         let netuid_yuma: u16 = 1; // holds 45% of the networks stake
//         let stake_yuma: u64 = to_nano(450_000);

//         let netuid_below_threshold: u16 = 2; // holds 5% of the networks stake
//         let stake_below_threshold: u64 = to_nano(50_000);

//         // making sure the unit emission are set correctly
//         UnitEmission::<Test>::put(23148148148);
//         zero_min_burn();
//         SubnetStakeThreshold::<Test>::put(Percent::from_percent(10));
//         let blocks_in_day: u16 = 10_800;
//         // this is aprox. the stake we expect at the end of the day with the above unit emission
//         let expected_stake_change = to_nano(250_000);
//         let expected_stake_change_general = (stake_general as f64
//             / ((stake_general + stake_yuma) as f64)
//             * expected_stake_change as f64) as u64;
//         let expected_stake_change_yuma = (stake_yuma as f64 / ((stake_general + stake_yuma) as
// f64)
//             * expected_stake_change as f64) as u64;
//         let expected_stake_change_below = 0;
//         let change_tolerance = to_nano(22) as i64; // we tolerate 22 token difference (due to
// rounding)

//         // first register the general subnet
//         assert_ok!(register_module(
//             netuid_general,
//             U256::from(0),
//             stake_general
//         ));
//         FounderShare::<Test>::set(netuid_general, 0);

//         // then register the yuma subnet
//         assert_ok!(register_module(netuid_yuma, U256::from(1), stake_yuma));

//         // then register the below threshold subnet
//         assert_ok!(register_module(
//             netuid_below_threshold,
//             U256::from(2),
//             stake_below_threshold
//         ));

//         FounderShare::<Test>::set(0, 0);
//         FounderShare::<Test>::set(1, 0);
//         FounderShare::<Test>::set(2, 0);

//         step_block(blocks_in_day);

//         let general_netuid_stake =
// from_nano(SubspaceMod::get_total_subnet_stake(netuid_general));         let yuma_netuid_stake =
// from_nano(SubspaceMod::get_total_subnet_stake(netuid_yuma));         let
// below_threshold_netuid_stake =
// from_nano(SubspaceMod::get_total_subnet_stake(netuid_below_threshold));

//         let general_netuid_stake = (general_netuid_stake as f64 / 100.0).round() * 100.0;
//         let yuma_netuid_stake = (yuma_netuid_stake as f64 / 100.0).round() * 100.0;
//         let below_threshold_netuid_stake =
//             (below_threshold_netuid_stake as f64 / 100.0).round() * 100.0;

//         let start_stake = stake_general + stake_yuma + stake_below_threshold;
//         let end_day_stake = to_nano(
//             (general_netuid_stake + yuma_netuid_stake + below_threshold_netuid_stake) as u64,
//         );
//         let stake_change = end_day_stake - start_stake;
//         assert_eq!(stake_change, expected_stake_change);

//         // Check the expected difference for the general subnet
//         let general_stake_change = to_nano(general_netuid_stake as u64) - stake_general;
//         assert!(
//             (general_stake_change as i64 - expected_stake_change_general as i64).abs()
//                 <= change_tolerance
//         );

//         // Check the expected difference for the yuma subnet
//         let yuma_stake_change = to_nano(yuma_netuid_stake as u64) - stake_yuma;
//         assert!(
//             (yuma_stake_change as i64 - expected_stake_change_yuma as i64).abs()
//                 <= change_tolerance
//         );

//         // Check the expected difference for the below threshold subnet
//         let below_stake_change =
//             to_nano(below_threshold_netuid_stake as u64) - stake_below_threshold;
//         assert_eq!(below_stake_change, expected_stake_change_below);
//     });
// }

#[test]
#[ignore = "global stake?"]
fn test_yuma_self_vote() {
    new_test_ext().execute_with(|| {
        let netuid_general: u16 = 0;
        let netuid_yuma: u16 = 1;
        let netuid_below_threshold: u16 = 2;
        // this much stake is on the general subnet 0
        let stake_general: u64 = to_nano(500_000);
        // this is how much the first voter on yuma consensus has
        let stake_yuma_voter: u64 = to_nano(440_000);
        // miner
        let stake_yuma_miner: u64 = to_nano(10_000);
        // this is how much the self voter on yuma consensus has
        let stake_yuma_voter_self: u64 = to_nano(400_000);
        let stake_yuma_miner_self: u64 = to_nano(2_000);
        // below threshold subnet, emission distribution should not even start
        let stake_below_threshold: u64 = to_nano(50_000);
        let blocks_in_day: u16 = 10_800;
        let validator_key = U256::from(1);
        let miner_key = U256::from(2);
        let validator_self_key = U256::from(3);
        let miner_self_key = U256::from(4);

        // making sure the unit emission are set correctly
        UnitEmission::<Test>::put(23148148148);
        zero_min_burn();

        assert_ok!(register_module(
            netuid_general,
            U256::from(0),
            stake_general
        ));
        FounderShare::<Test>::set(netuid_general, 0);
        assert_ok!(register_module(
            netuid_yuma,
            validator_key,
            stake_yuma_voter
        ));
        update_params!(netuid_yuma => { max_weight_age: (blocks_in_day + 1) as u64});
        assert_ok!(register_module(netuid_yuma, miner_key, stake_yuma_miner));
        assert_ok!(register_module(
            netuid_yuma,
            validator_self_key,
            stake_yuma_voter_self
        ));
        assert_ok!(register_module(
            netuid_yuma,
            miner_self_key,
            stake_yuma_miner_self
        ));
        step_block(1);
        set_weights(
            netuid_yuma,
            validator_key,
            [SubspaceMod::get_uid_for_key(netuid_yuma, &miner_key)].to_vec(),
            [1].to_vec(),
        );
        set_weights(
            netuid_yuma,
            validator_self_key,
            [SubspaceMod::get_uid_for_key(netuid_yuma, &miner_self_key)].to_vec(),
            [1].to_vec(),
        );
        assert_ok!(register_module(
            netuid_below_threshold,
            U256::from(2),
            stake_below_threshold
        ));

        // Calculate the expected daily change in total stake
        let expected_stake_change = to_nano(250_000);

        FounderShare::<Test>::set(0, 0);
        FounderShare::<Test>::set(1, 0);
        FounderShare::<Test>::set(2, 0);

        step_block(blocks_in_day);

        let stake_validator = Stake::<Test>::get(&validator_key);
        let stake_miner = Stake::<Test>::get(&miner_key);
        let stake_validator_self_vote = Stake::<Test>::get(&validator_self_key);
        let stake_miner_self_vote = Stake::<Test>::get(&miner_self_key);

        assert!(stake_yuma_voter < stake_validator);
        assert!(stake_yuma_miner < stake_miner);
        assert_eq!(stake_yuma_miner_self, stake_miner_self_vote);
        assert_eq!(stake_yuma_voter_self, stake_validator_self_vote);

        let general_netuid_stake = SubspaceMod::get_total_subnet_stake(netuid_general);
        let yuma_netuid_stake = SubspaceMod::get_total_subnet_stake(netuid_yuma);
        let below_threshold_netuid_stake =
            SubspaceMod::get_total_subnet_stake(netuid_below_threshold);

        assert!(stake_general < general_netuid_stake);
        assert!(stake_yuma_voter < yuma_netuid_stake);
        assert_eq!(stake_below_threshold, below_threshold_netuid_stake);
        // Check the actual daily change in total stake
        let start_stake = stake_below_threshold
            + stake_general
            + stake_yuma_voter
            + stake_yuma_voter_self
            + stake_yuma_miner
            + stake_yuma_miner_self;
        let end_day_stake = general_netuid_stake + yuma_netuid_stake + below_threshold_netuid_stake;
        let actual_stake_change = round_first_five(end_day_stake - start_stake);

        assert_eq!(actual_stake_change, expected_stake_change);
    });
}

// FIXME:
// Global Stake
// #[test]
// fn test_emission_activation() {
//     new_test_ext().execute_with(|| {
//         // Define the subnet stakes
//         let subnet_stakes = [
//             ("Subnet A", to_nano(10), true),
//             ("Subnet B", to_nano(4), false), // This one should not activate
//             ("Subnet C", to_nano(86), true),
//         ];

//         // Set the stake threshold and minimum burn
//         SubnetStakeThreshold::<Test>::put(Percent::from_percent(5));
//         zero_min_burn();

//         // Register the subnets
//         for (i, (name, stake, _)) in subnet_stakes.iter().enumerate() {
//             assert_ok!(register_module(i as u16, U256::from(i as u64), *stake));
//             info!("Registered {name} with stake: {stake}");
//         }

//         step_block(1_000);

//         // Check if subnet rewards have increased, but Subnet B should not have activated
//         for (i, (name, initial_stake, should_activate)) in subnet_stakes.iter().enumerate() {
//             let current_stake = SubspaceMod::get_total_subnet_stake(i as u16);
//             if *should_activate {
//                 assert!(
//                     current_stake > *initial_stake,
//                     "{name} should have activated and increased its stake"
//                 );
//             } else {
//                 assert_eq!(
//                     current_stake, *initial_stake,
//                     "{name} should not have activated"
//                 );
//             }
//             info!("{name} current stake: {current_stake}");
//         }
//     });
// }

// immunity period attack
// this test should ignore, immunity period of subnet under specific conditions
#[test]
#[ignore = "global stake ?"]
fn test_parasite_subnet_registrations() {
    new_test_ext().execute_with(|| {
        let expected_module_amount: u16 = 5;
        MaxAllowedModules::<Test>::put(expected_module_amount);
        MaxRegistrationsPerBlock::<Test>::set(1000);

        let main_subnet_netuid: u16 = 0;
        let main_subnet_stake = to_nano(500_000);
        let main_subnet_key = U256::from(0);

        let parasite_netuid: u16 = 1;
        let parasite_subnet_stake = to_nano(1_000);
        let parasite_subnet_key = U256::from(1);

        // Register the honest subnet.
        assert_ok!(register_module(
            main_subnet_netuid,
            main_subnet_key,
            main_subnet_stake
        ));
        // Set the immunity period of the honest subnet to 1000 blocks.
        update_params!(main_subnet_netuid => { immunity_period: 1000 });

        // Register the parasite subnet
        assert_ok!(register_module(
            parasite_netuid,
            parasite_subnet_key,
            parasite_subnet_stake
        ));
        // Parasite subnet set it's immunity period to 100k blocks.
        update_params!(parasite_netuid => { immunity_period: u16::MAX });

        // Honest subnet will now register another module, so it will have 2 in total.
        assert_ok!(register_module(
            main_subnet_netuid,
            U256::from(2),
            main_subnet_stake
        ));

        // Parasite subnet will now try to register a large number of modules.
        // This is in hope of deregistering modules from the honest subnet.
        for i in 10..50 {
            let result = register_module(parasite_netuid, U256::from(i), parasite_subnet_stake);
            assert_ok!(result);
        }

        // Check if the honest subnet has 2 modules.
        let main_subnet_module_amount = N::<Test>::get(main_subnet_netuid);
        assert_eq!(main_subnet_module_amount, 2);

        // Check if the parasite subnet has 3 modules
        let parasite_subnet_module_amount = N::<Test>::get(parasite_netuid);
        assert_eq!(parasite_subnet_module_amount, 3);
    });
}

// After reaching maximum global modules, subnets will start getting deregisterd
// Test ensures that newly registered subnets will take the "spots" of these deregistered subnets.
// And modules go beyond the global maximum.
#[test]
#[ignore = "global stake ?"]
fn test_subnet_replacing() {
    new_test_ext().execute_with(|| {
        // Defines the maximum number of modules, that can be registered,
        // on all subnets at once.
        let expected_subnet_amount: u16 = 3;
        MaxAllowedModules::<Test>::put(expected_subnet_amount);

        let subnets = [
            (U256::from(0), to_nano(100_000)),
            (U256::from(1), to_nano(5000)),
            (U256::from(2), to_nano(4_000)),
            (U256::from(3), to_nano(1_100)),
        ];

        let random_keys = [U256::from(4), U256::from(5)];

        // Register all subnets
        for (i, (subnet_key, subnet_stake)) in subnets.iter().enumerate() {
            assert_ok!(register_module(i as u16, *subnet_key, *subnet_stake));
        }

        let subnet_amount = TotalSubnets::<Test>::get();
        assert_eq!(subnet_amount, expected_subnet_amount);

        // Register module on the subnet one (netuid 0), this means that subnet
        // subnet two (netuid 1) will be deregistered, as we reached global module limit.
        assert_ok!(register_module(0, random_keys[0], to_nano(1_000)));
        assert_ok!(register_module(4, random_keys[1], to_nano(150_000)));

        let subnet_amount = TotalSubnets::<Test>::get();
        assert_eq!(subnet_amount, expected_subnet_amount - 1);

        // netuid 1 replaced by subnet four
        assert_ok!(register_module(3, subnets[3].0, subnets[3].1));

        let subnet_amount = TotalSubnets::<Test>::get();
        let total_module_amount = SubspaceMod::global_n_modules();
        assert_eq!(subnet_amount, expected_subnet_amount);
        assert_eq!(total_module_amount, expected_subnet_amount);

        let netuids = SubspaceMod::netuids();
        let max_netuid = netuids.iter().max().unwrap();
        assert_eq!(*max_netuid, 2);
    });
}

// FIXME:
// GLOBAL STAKE
// #[test]
// fn test_active_stake() {
//     new_test_ext().execute_with(|| {
//         SubnetStakeThreshold::<Test>::put(Percent::from_percent(5));
//         let max_subnets = 10;
//         MaxAllowedSubnets::<Test>::put(max_subnets);

//         let general_subnet_stake = to_nano(65_000_000);
//         let general_subnet_key = U256::from(0);
//         assert_ok!(register_module(0, general_subnet_key, general_subnet_stake));
//         step_block(1);
//         // register 9 subnets reaching the subnet limit,
//         // make sure they can't get emission
//         let n: u16 = 9;
//         let stake_per_subnet: u64 = to_nano(8_001);
//         for i in 1..n + 1 {
//             assert_ok!(register_module(
//                 i,
//                 U256::from(i),
//                 stake_per_subnet - (i as u64 * 1000)
//             ));
//             step_block(1)
//         }

//         for i in 0..max_subnets {
//             assert_eq!(N::<Test>::get(i), 1);
//         }

//         step_block(200);

//         // deregister subnet netuid 9, and get enough emission to produce yuma
//         let new_subnet_stake = to_nano(9_900_000);
//         assert_ok!(register_module(10, U256::from(10), new_subnet_stake));
//         step_block(7);

//         for i in 0..max_subnets {
//             assert_eq!(N::<Test>::get(i), 1);
//         }
//         assert!(SubspaceMod::is_registered(9, &U256::from(10)));

//         // register another module on the newly re-registered subnet 9,
//         // and set weights on it from the key 11
//         let miner_key = U256::from(11);
//         let miner_stake = to_nano(100_000);
//         assert_ok!(register_module(10, miner_key, miner_stake));

//         step_block(1);

//         assert_eq!(N::<Test>::get(9), 2);

//         // set weights from key 11 to miner
//         let uids = [1].to_vec();
//         let weights = [1].to_vec();

//         set_weights(9, U256::from(10), uids, weights);

//         step_block(100);

//         // register another massive module on the subnet
//         let new_module_stake = to_nano(9_000_000);
//         assert_ok!(register_module(10, U256::from(12), new_module_stake));

//         step_block(1);
//         assert!(SubspaceMod::is_registered(Some((9)), &U256::from(12)));
//         // check if the module is registered
//         assert_eq!(N::<Test>::get(9), 3);

//         // set weights from key 12 to both modules
//         let uids = [0, 1].to_vec();
//         let weights = [1, 1].to_vec();

//         set_weights(9, U256::from(12), uids, weights);

//         let n = 10;
//         let stake_per_n = to_nano(20_000_000);
//         let start_key = 13;
//         // register the n modules
//         for i in 0..n {
//             assert_ok!(register_module(10, U256::from(i + start_key), stake_per_n));
//         }

//         assert_eq!(N::<Test>::get(9), 3 + n);
//         step_block(100);

//         dbg!(Dividends::<Test>::get(9));
//         let uid_zero_dividends = SubspaceMod::get_dividends_for_uid(9, 0);
//         let uid_two_dividends = SubspaceMod::get_dividends_for_uid(9, 2);
//         let total_dividends_sum = Dividends::<Test>::get(9).iter().sum::<u16>();
//         let active_dividends = uid_zero_dividends + uid_two_dividends;

//         assert!(uid_zero_dividends > 0);
//         assert!(uid_two_dividends > 0);
//         assert!(uid_zero_dividends > uid_two_dividends);
//         assert_eq!(total_dividends_sum, active_dividends);
//     });
// }

// this test, should confirm that we can update subnet using the same name
#[test]
fn test_update_same_name() {
    new_test_ext().execute_with(|| {
        // general subnet
        let netuid: u16 = 0;
        let key = U256::from(0);
        let stake = to_nano(100_000);

        assert_ok!(register_module(netuid, key, stake));

        assert_eq!(N::<Test>::get(0), 1);

        dbg!(SubnetNames::<Test>::get(netuid));
        let mut params = SubspaceMod::subnet_params(netuid);
        let new_tempo = 30;
        params.tempo = new_tempo;
        let result = SubspaceMod::update_subnet(
            get_origin(key),
            netuid,
            params.founder,
            params.founder_share,
            params.immunity_period,
            params.incentive_ratio,
            params.max_allowed_uids,
            params.max_allowed_weights,
            params.min_allowed_weights,
            params.max_weight_age,
            params.min_stake,
            params.name,
            params.tempo,
            params.trust_ratio,
            params.maximum_set_weight_calls_per_epoch,
            params.vote_mode,
            params.bonds_ma,
        );

        dbg!(SubnetNames::<Test>::get(netuid));
        assert_ok!(result);
        let new_params = SubspaceMod::subnet_params(netuid);
        assert_eq!(new_params.tempo, new_tempo);
    });
}

#[test]
fn test_set_weight_rate_limiting() {
    new_test_ext().execute_with(|| {
        let netuid: u16 = 0;
        let key = U256::from(0);
        let stake = to_nano(100_000);

        assert_ok!(register_module(netuid, key, stake));
        assert_ok!(register_module(netuid, 1.into(), stake));

        Tempo::<Test>::set(netuid, 5);

        MaximumSetWeightCallsPerEpoch::<Test>::set(netuid, 1);

        let set_weights = || SubspaceMod::set_weights(get_origin(key), netuid, vec![1], vec![10]);

        assert_ok!(set_weights());
        assert_err!(
            set_weights(),
            Error::<Test>::MaximumSetWeightsPerEpochReached
        );

        step_block(5);

        assert_ok!(set_weights());
        assert_err!(
            set_weights(),
            Error::<Test>::MaximumSetWeightsPerEpochReached
        );

        MaximumSetWeightCallsPerEpoch::<Test>::set(netuid, 0);

        assert_ok!(set_weights());
        assert_ok!(set_weights());
        assert_ok!(set_weights());
        assert_ok!(set_weights());
    });
}

// Voting

#[test]
#[ignore = "global stake?"]
fn creates_global_params_proposal_correctly_and_expires() {
    new_test_ext().execute_with(|| {
        const COST: u64 = to_nano(10);

        zero_min_burn();
        FloorFounderShare::<Test>::set(0);

        ProposalCost::<Test>::set(COST);
        ProposalExpiration::<Test>::set(100);

        step_block(1);

        let key = U256::from(0);
        SubspaceMod::add_balance_to_account(&key, COST + 1);
        assert_ok!(register_module(0, U256::from(1), 1_000_000_000));
        assert_ok!(register_module(0, U256::from(2), 1_000_000_100));

        let burn_config = BurnConfiguration {
            min_burn: 100_000_000,
            ..BurnConfiguration::<Test>::default()
        };
        assert_ok!(burn_config.apply());

        let original = SubspaceMod::global_params();

        let BurnConfiguration {
            min_burn,
            max_burn,
            adjustment_alpha,
            adjustment_interval,
            expected_registrations,
            ..
        } = BurnConfig::<Test>::get();

        let GlobalParams {
            max_name_length,
            min_name_length,
            max_allowed_subnets,
            max_allowed_modules,
            max_registrations_per_block,
            max_allowed_weights,
            floor_delegation_fee,
            floor_founder_share,
            min_weight_stake,
            curator,
            proposal_cost,
            proposal_expiration,
            proposal_participation_threshold,
            general_subnet_application_cost,
            ..
        } = original.clone();

        SubspaceMod::add_global_proposal(
            get_origin(key),
            max_name_length,
            min_name_length,
            max_allowed_subnets,
            max_allowed_modules,
            max_registrations_per_block,
            max_allowed_weights,
            max_burn,
            min_burn,
            floor_delegation_fee,
            floor_founder_share,
            min_weight_stake,
            expected_registrations,
            adjustment_interval,
            adjustment_alpha,
            curator,
            proposal_cost,
            proposal_expiration,
            proposal_participation_threshold,
            general_subnet_application_cost,
        )
        .expect("failed to create proposal");

        assert_eq!(SubspaceMod::get_balance_u64(&key), 1);

        let proposal = Proposals::<Test>::get(0).expect("proposal was not created");
        assert_eq!(proposal.id, 0);
        assert_eq!(proposal.proposer, key);
        assert_eq!(proposal.expiration_block, 200);
        assert_eq!(
            proposal.data,
            ProposalData::<Test>::GlobalParams(original.clone())
        );
        assert_eq!(proposal.status, ProposalStatus::Pending);
        assert_eq!(proposal.votes_for, Default::default());
        assert_eq!(proposal.votes_against, Default::default());
        assert_eq!(proposal.proposal_cost, COST);
        assert_eq!(proposal.finalization_block, None);

        SubspaceMod::vote_proposal(get_origin(U256::from(1)), 0, true).unwrap();

        step_block(200);

        assert_eq!(SubspaceMod::get_balance_u64(&key), 1);

        assert_eq!(SubspaceMod::global_params(), original);
    });
}

#[test]
#[ignore = "global stake ??"]
fn creates_global_params_proposal_correctly_and_is_approved() {
    new_test_ext().execute_with(|| {
        const COST: u64 = to_nano(10);

        zero_min_burn();

        let keys: [_; 3] = from_fn(U256::from);
        let stakes = [1_000_000_000, 1_000_000_000, 1_000_000_000];

        for (key, balance) in keys.iter().zip(stakes) {
            assert_ok!(register_module(0, *key, balance));
        }
        SubspaceMod::add_balance_to_account(&keys[0], COST);

        ProposalCost::<Test>::set(COST);
        ProposalExpiration::<Test>::set(200);

        let burn_config = BurnConfiguration {
            min_burn: 100_000_000,
            ..BurnConfiguration::<Test>::default()
        };
        assert_ok!(burn_config.apply());

        let BurnConfiguration {
            min_burn,
            max_burn,
            adjustment_alpha,
            adjustment_interval,
            expected_registrations,
            ..
        } = BurnConfig::<Test>::get();

        let params = SubspaceMod::global_params();

        let GlobalParams {
            max_name_length,
            min_name_length,
            max_allowed_subnets,
            max_allowed_modules,
            max_registrations_per_block,
            max_allowed_weights,
            floor_delegation_fee,
            floor_founder_share,
            min_weight_stake,
            curator,
            proposal_cost,
            proposal_expiration,
            proposal_participation_threshold,
            general_subnet_application_cost,
            ..
        } = params.clone();

        SubspaceMod::add_global_proposal(
            get_origin(keys[0]),
            max_name_length,
            min_name_length,
            max_allowed_subnets,
            max_allowed_modules,
            max_registrations_per_block,
            max_allowed_weights,
            max_burn,
            min_burn,
            floor_delegation_fee,
            floor_founder_share,
            min_weight_stake,
            expected_registrations,
            adjustment_interval,
            adjustment_alpha,
            curator,
            proposal_cost,
            proposal_expiration,
            proposal_participation_threshold,
            general_subnet_application_cost,
        )
        .expect("failed to create proposal");

        assert_eq!(SubspaceMod::get_balance_u64(&keys[0]), 1);

        SubspaceMod::vote_proposal(get_origin(keys[0]), 0, true).unwrap();
        SubspaceMod::vote_proposal(get_origin(keys[1]), 0, true).unwrap();
        SubspaceMod::vote_proposal(get_origin(keys[2]), 0, false).unwrap();

        ProposalCost::<Test>::set(COST * 2);

        step_block(100);

        let proposal = Proposals::<Test>::get(0).expect("proposal was not created");
        assert_eq!(proposal.status, ProposalStatus::Accepted);
        assert_eq!(proposal.finalization_block, Some(100));
        assert_eq!(
            SubspaceMod::get_balance_u64(&keys[0]),
            proposal.proposal_cost + 1,
        );

        ProposalCost::<Test>::set(COST);
        assert_eq!(SubspaceMod::global_params(), params);
    });
}

#[test]
#[ignore = "global stake ?"]
fn creates_global_params_proposal_correctly_and_is_refused() {
    new_test_ext().execute_with(|| {
        const COST: u64 = to_nano(10);

        zero_min_burn();

        let keys: [_; 3] = from_fn(U256::from);
        let stakes = [1_000_000_000, 1_000_000_000, 1_000_000_000];

        for (key, balance) in keys.iter().zip(stakes) {
            assert_ok!(register_module(0, *key, balance));
        }
        SubspaceMod::add_balance_to_account(&keys[0], COST);

        ProposalCost::<Test>::set(COST);
        ProposalExpiration::<Test>::set(200);

        let burn_config = BurnConfiguration {
            min_burn: 100_000_000,
            ..BurnConfiguration::<Test>::default()
        };
        assert_ok!(burn_config.apply());

        let original = SubspaceMod::global_params();
        let GlobalParams {
            floor_founder_share,
            max_name_length,
            min_name_length,
            max_allowed_subnets,
            max_allowed_modules,
            max_registrations_per_block,
            max_allowed_weights,
            floor_delegation_fee,
            min_weight_stake,
            curator,
            proposal_cost,
            proposal_expiration,
            proposal_participation_threshold,
            general_subnet_application_cost,
            ..
        } = GlobalParams { ..original.clone() };

        let BurnConfiguration {
            min_burn,
            max_burn,
            adjustment_alpha,
            adjustment_interval,
            expected_registrations,
            ..
        } = BurnConfig::<Test>::get();

        SubspaceMod::add_global_proposal(
            get_origin(keys[0]),
            max_name_length,
            min_name_length,
            max_allowed_subnets,
            max_allowed_modules,
            max_registrations_per_block,
            max_allowed_weights,
            max_burn,
            min_burn,
            floor_delegation_fee,
            floor_founder_share,
            min_weight_stake,
            expected_registrations,
            adjustment_interval,
            adjustment_alpha,
            curator,
            proposal_cost,
            proposal_expiration,
            proposal_participation_threshold,
            general_subnet_application_cost,
        )
        .expect("failed to create proposal");

        assert_eq!(SubspaceMod::get_balance_u64(&keys[0]), 1);

        SubspaceMod::vote_proposal(get_origin(keys[0]), 0, true).unwrap();
        SubspaceMod::vote_proposal(get_origin(keys[1]), 0, false).unwrap();
        SubspaceMod::vote_proposal(get_origin(keys[2]), 0, false).unwrap();

        ProposalCost::<Test>::set(COST * 2);

        step_block(100);

        let proposal = Proposals::<Test>::get(0).expect("proposal was not created");
        assert_eq!(proposal.status, ProposalStatus::Refused);
        assert_eq!(proposal.finalization_block, Some(100));
        assert_eq!(SubspaceMod::get_balance_u64(&keys[0]), 1,);

        ProposalCost::<Test>::set(COST);
        assert_eq!(SubspaceMod::global_params(), original);
    });
}

#[test]
#[ignore = "global stake ?"]
fn creates_subnet_params_proposal_correctly_and_is_approved() {
    new_test_ext().execute_with(|| {
        const COST: u64 = to_nano(10);

        zero_min_burn();

        let keys: [_; 3] = from_fn(U256::from);
        let stakes = [1_000_000_000, 1_000_000_000, 1_000_000_000];

        for (key, balance) in keys.iter().zip(stakes) {
            assert_ok!(register_module(0, *key, balance));
        }
        SubspaceMod::add_balance_to_account(&keys[0], COST);

        ProposalCost::<Test>::set(COST);
        ProposalExpiration::<Test>::set(200);
        VoteModeSubnet::<Test>::set(0, VoteMode::Vote);

        let params = SubnetParams {
            tempo: 150,
            ..SubspaceMod::subnet_params(0)
        };

        let SubnetParams {
            founder,
            name,
            founder_share,
            immunity_period,
            incentive_ratio,
            max_allowed_uids,
            max_allowed_weights,
            min_allowed_weights,
            min_stake,
            max_weight_age,
            tempo,
            trust_ratio,
            maximum_set_weight_calls_per_epoch,
            vote_mode,
            bonds_ma,
        } = params.clone();

        SubspaceMod::add_subnet_proposal(
            get_origin(keys[0]),
            0, // netuid
            founder,
            name,
            founder_share,
            immunity_period,
            incentive_ratio,
            max_allowed_uids,
            max_allowed_weights,
            min_allowed_weights,
            min_stake,
            max_weight_age,
            tempo,
            trust_ratio,
            maximum_set_weight_calls_per_epoch,
            vote_mode,
            bonds_ma,
        )
        .expect("failed to create proposal");

        assert_eq!(SubspaceMod::get_balance_u64(&keys[0]), 1);

        SubspaceMod::vote_proposal(get_origin(keys[0]), 0, true).unwrap();
        SubspaceMod::vote_proposal(get_origin(keys[1]), 0, true).unwrap();
        SubspaceMod::vote_proposal(get_origin(keys[2]), 0, false).unwrap();

        ProposalCost::<Test>::set(COST * 2);

        step_block(100);

        let proposal = Proposals::<Test>::get(0).expect("proposal was not created");
        assert_eq!(proposal.status, ProposalStatus::Accepted);
        assert_eq!(proposal.finalization_block, Some(100));
        assert_eq!(
            SubspaceMod::get_balance_u64(&keys[0]),
            proposal.proposal_cost + 1,
        );

        dbg!(Tempo::<Test>::contains_key(0));

        ProposalCost::<Test>::set(COST);
        assert_eq!(SubspaceMod::subnet_params(0).tempo, 150);
    });
}

#[test]
fn unregister_vote_from_pending_proposal() {
    new_test_ext().execute_with(|| {
        const COST: u64 = to_nano(10);

        zero_min_burn();

        let key = U256::from(0);
        assert_ok!(register_module(0, key, 1_000_000_000));
        SubspaceMod::add_balance_to_account(&key, COST);

        ProposalCost::<Test>::set(COST);

        SubspaceMod::add_custom_proposal(get_origin(key), b"test".to_vec())
            .expect("failed to create proposal");

        SubspaceMod::vote_proposal(get_origin(key), 0, true).unwrap();
        let proposal = Proposals::<Test>::get(0).expect("proposal was not created");
        assert_eq!(proposal.votes_for, BTreeSet::from([key]));

        SubspaceMod::unvote_proposal(get_origin(key), 0).unwrap();
        let proposal = Proposals::<Test>::get(0).expect("proposal was not created");
        assert_eq!(proposal.votes_for, BTreeSet::from([]));
    });
}

#[test]
fn fails_if_insufficient_dao_treasury_fund() {
    new_test_ext().execute_with(|| {
        const COST: u64 = to_nano(10);
        zero_min_burn();
        ProposalCost::<Test>::set(COST);
        SubspaceMod::add_balance_to_account(&DaoTreasuryAddress::<Test>::get(), 10);

        let key = U256::from(0);
        assert_ok!(register_module(0, key, 1_000_000_000));
        SubspaceMod::add_balance_to_account(&key, COST);

        let origin = get_origin(key);
        assert_err!(
            SubspaceMod::add_transfer_dao_treasury_proposal(origin, vec![0], 11, key),
            Error::<Test>::InsufficientDaoTreasuryFunds
        )
    });
}

// Weights
//========

#[test]
fn test_weights_err_weights_vec_not_equal_size() {
    new_test_ext().execute_with(|| {
        let netuid: u16 = 0;
        let key_account_id = U256::from(55);
        // make sure that the results won´t get affected by burn
        zero_min_burn();
        assert_ok!(register_module(netuid, key_account_id, 1_000_000_000));
        let weights_keys: Vec<u16> = vec![1, 2, 3, 4, 5, 6];
        let weight_values: Vec<u16> = vec![1, 2, 3, 4, 5]; // Uneven sizes
        let result = SubspaceMod::set_weights(
            RuntimeOrigin::signed(key_account_id),
            netuid,
            weights_keys,
            weight_values,
        );
        assert_eq!(result, Err(Error::<Test>::WeightVecNotEqualSize.into()));
    });
}

// Test ensures that uids can have not duplicates
#[test]
fn test_weights_err_has_duplicate_ids() {
    new_test_ext().execute_with(|| {
        let key_account_id = U256::from(666);
        let netuid: u16 = 0;
        // make sure that the results won´t get affected by burn
        zero_min_burn();
        MaxRegistrationsPerBlock::<Test>::set(100);

        assert_ok!(register_module(netuid, key_account_id, 10));
        update_params!(netuid => { max_allowed_uids: 100 });

        // uid 1
        assert_ok!(register_module(netuid, U256::from(1), 100));
        SubspaceMod::get_uid_for_key(netuid, &U256::from(1));

        // uid 2
        assert_ok!(register_module(netuid, U256::from(2), 10000));
        SubspaceMod::get_uid_for_key(netuid, &U256::from(2));

        // uid 3
        assert_ok!(register_module(netuid, U256::from(3), 10000000));
        SubspaceMod::get_uid_for_key(netuid, &U256::from(3));

        assert_eq!(N::<Test>::get(netuid), 4);

        let weights_keys: Vec<u16> = vec![1, 1, 1]; // Contains duplicates
        let weight_values: Vec<u16> = vec![1, 2, 3];
        let result = SubspaceMod::set_weights(
            RuntimeOrigin::signed(key_account_id),
            netuid,
            weights_keys,
            weight_values,
        );
        assert_eq!(result, Err(Error::<Test>::DuplicateUids.into()));
    });
}

// Tests the call requires a valid origin.
#[test]
fn test_no_signature() {
    new_test_ext().execute_with(|| {
        let uids: Vec<u16> = vec![];
        let values: Vec<u16> = vec![];
        let result = SubspaceMod::set_weights(RuntimeOrigin::none(), 1, uids, values);
        assert_eq!(result, Err(DispatchError::BadOrigin));
    });
}

// Tests that set weights fails if you pass invalid uids.
#[test]
fn test_set_weights_err_invalid_uid() {
    new_test_ext().execute_with(|| {
        let key_account_id = U256::from(55);
        let netuid: u16 = 0;
        // make sure that the results won´t get affected by burn
        zero_min_burn();
        assert_ok!(register_module(netuid, key_account_id, 1_000_000_000));
        let weight_keys: Vec<u16> = vec![9999]; // Does not exist
        let weight_values: Vec<u16> = vec![88]; // random value
        let result = SubspaceMod::set_weights(
            RuntimeOrigin::signed(key_account_id),
            netuid,
            weight_keys,
            weight_values,
        );
        assert_eq!(result, Err(Error::<Test>::InvalidUid.into()));
    });
}

// Tests that set weights fails if you dont pass enough values.
#[test]
fn test_set_weight_not_enough_values() {
    new_test_ext().execute_with(|| {
        let netuid: u16 = 0;
        let n = 100;
        MaxRegistrationsPerBlock::<Test>::set(n);
        let account_id = U256::from(0);
        // make sure that the results won´t get affected by burn
        zero_min_burn();
        assert_ok!(register_module(netuid, account_id, 1_000_000_000));

        for i in 1..n {
            assert_ok!(register_module(netuid, U256::from(i), 1_000_000_000));
        }

        update_params!(netuid => { min_allowed_weights: 2 });

        // setting weight below minimim
        let weight_keys: Vec<u16> = vec![1]; // not weight.
        let weight_values: Vec<u16> = vec![88]; // random value.
        let result = SubspaceMod::set_weights(
            RuntimeOrigin::signed(account_id),
            netuid,
            weight_keys,
            weight_values,
        );
        assert_eq!(result, Err(Error::<Test>::InvalidUidsLength.into()));

        update_params!(netuid => { min_allowed_weights: 1 });

        let weight_keys: Vec<u16> = vec![0]; // self weight.
        let weight_values: Vec<u16> = vec![88]; // random value.
        let result = SubspaceMod::set_weights(
            RuntimeOrigin::signed(account_id),
            netuid,
            weight_keys,
            weight_values,
        );
        assert_eq!(result, Err(Error::<Test>::NoSelfWeight.into()));

        // Should pass because we are setting enough values.
        let weight_keys: Vec<u16> = vec![1, 2]; // self weight.
        let weight_values: Vec<u16> = vec![10, 10]; // random value.
        update_params!(netuid => { min_allowed_weights: 1 });
        assert_ok!(SubspaceMod::set_weights(
            RuntimeOrigin::signed(account_id),
            netuid,
            weight_keys,
            weight_values
        ));
    });
}

// Tests that set weights fails if you dont pass enough values.
#[test]
fn test_set_max_allowed_uids() {
    new_test_ext().execute_with(|| {
        let netuid: u16 = 0;
        let n = 100;
        MaxRegistrationsPerBlock::<Test>::set(n);
        let account_id = U256::from(0);
        // make sure that the results won´t get affected by burn
        zero_min_burn();
        assert_ok!(register_module(netuid, account_id, 1_000_000_000));
        for i in 1..n {
            assert_ok!(register_module(netuid, U256::from(i), 1_000_000_000));
        }

        let max_allowed_uids: u16 = 10;

        update_params!(netuid => { max_allowed_weights: max_allowed_uids });

        // Should fail because we are only setting a single value and its not the self weight.
        let weight_keys: Vec<u16> = (1..max_allowed_uids + 1).collect(); // not weight.
        let weight_values: Vec<u16> = vec![1; max_allowed_uids as usize]; // random value.
        let result = SubspaceMod::set_weights(
            RuntimeOrigin::signed(account_id),
            netuid,
            weight_keys,
            weight_values,
        );
        assert_ok!(result);
    });
}

/// Check do nothing path
#[test]
fn test_normalize_weights_does_not_mutate_when_sum_is_zero() {
    new_test_ext().execute_with(|| {
        let max_allowed: u16 = 3;

        let weights: Vec<u16> = Vec::from_iter((0..max_allowed).map(|_| 0));

        let _expected = weights.clone();
        let _result = SubspaceMod::normalize_weights(weights);
    });
}

/// Check do something path
#[test]
fn test_normalize_weights_does_not_mutate_when_sum_not_zero() {
    new_test_ext().execute_with(|| {
        let max_allowed: u16 = 3;

        let weights: Vec<u16> = Vec::from_iter(0..max_allowed);

        let expected = weights.clone();
        let result = SubspaceMod::normalize_weights(weights);

        assert_eq!(expected.len(), result.len(), "Length of weights changed?!");
    });
}

#[test]
fn test_min_weight_stake() {
    new_test_ext().execute_with(|| {
        let mut global_params = SubspaceMod::global_params();
        global_params.min_weight_stake = to_nano(20);
        SubspaceMod::set_global_params(global_params);
        MaxRegistrationsPerBlock::<Test>::set(1000);

        let netuid: u16 = 0;
        let module_count: u16 = 16;
        let voter_idx: u16 = 0;

        // registers the modules
        for i in 0..module_count {
            assert_ok!(register_module(netuid, U256::from(i), to_nano(10)));
        }

        let uids: Vec<u16> = (0..module_count).filter(|&uid| uid != voter_idx).collect();
        let weights = vec![1; uids.len()];

        assert_err!(
            SubspaceMod::set_weights(
                get_origin(U256::from(voter_idx)),
                netuid,
                uids.clone(),
                weights.clone(),
            ),
            Error::<Test>::NotEnoughStakePerWeight
        );

        (SubspaceMod::increase_stake(&U256::from(voter_idx), &U256::from(voter_idx), to_nano(400)));

        assert_ok!(SubspaceMod::set_weights(
            get_origin(U256::from(voter_idx)),
            netuid,
            uids,
            weights,
        ));
    });
}

#[test]
fn test_weight_age() {
    new_test_ext().execute_with(|| {
        const NETUID: u16 = 0;
        const MODULE_COUNT: u16 = 16;
        const TEMPO: u64 = 100;
        const PASSIVE_VOTER: u16 = 0;
        const ACTIVE_VOTER: u16 = 1;
        MaxRegistrationsPerBlock::<Test>::set(1000);
        FloorFounderShare::<Test>::put(0);

        // Register modules
        (0..MODULE_COUNT).for_each(|i| {
            assert_ok!(register_module(NETUID, U256::from(i), to_nano(10)));
        });

        let uids: Vec<u16> = (0..MODULE_COUNT)
            .filter(|&uid| uid != PASSIVE_VOTER && uid != ACTIVE_VOTER)
            .collect();
        let weights = vec![1; uids.len()];

        // Set subnet parameters
        update_params!(NETUID => { tempo: TEMPO as u16, max_weight_age: TEMPO * 2 });

        // Set weights for passive and active voters
        assert_ok!(SubspaceMod::set_weights(
            get_origin(U256::from(PASSIVE_VOTER)),
            NETUID,
            uids.clone(),
            weights.clone(),
        ));
        assert_ok!(SubspaceMod::set_weights(
            get_origin(U256::from(ACTIVE_VOTER)),
            NETUID,
            uids.clone(),
            weights.clone(),
        ));

        let passive_stake_before = SubspaceMod::get_total_stake_to(&U256::from(PASSIVE_VOTER));
        let active_stake_before = SubspaceMod::get_total_stake_to(&U256::from(ACTIVE_VOTER));

        step_block((TEMPO as u16) * 2);

        let passive_stake_after = SubspaceMod::get_total_stake_to(&U256::from(PASSIVE_VOTER));
        let active_stake_after = SubspaceMod::get_total_stake_to(&U256::from(ACTIVE_VOTER));

        assert!(
            passive_stake_before < passive_stake_after || active_stake_before < active_stake_after,
            "Stake should be increasing"
        );

        // Set weights again for active voter
        assert_ok!(SubspaceMod::set_weights(
            get_origin(U256::from(ACTIVE_VOTER)),
            NETUID,
            uids,
            weights,
        ));

        step_block((TEMPO as u16) * 2);

        let passive_stake_after_v2 = SubspaceMod::get_total_stake_to(&U256::from(PASSIVE_VOTER));
        let active_stake_after_v2 = SubspaceMod::get_total_stake_to(&U256::from(ACTIVE_VOTER));

        assert_eq!(
            passive_stake_after, passive_stake_after_v2,
            "Stake values should remain the same after maximum weight age"
        );
        assert!(
            active_stake_after < active_stake_after_v2,
            "Stake should be increasing"
        );
    });
}

// Subnet Burn

#[test]
fn test_fail_if_no_subnet_burn() {
    new_test_ext().execute_with(|| {
        SubnetBurnConfig::<Test>::set(BurnConfiguration::<Test> {
            min_burn: 2_000_000_000_000,
            max_burn: 100_000_000_000_000,
            adjustment_alpha: (u64::MAX as f32 / 2f32 * 1.2) as u64,
            adjustment_interval: 2_000,
            expected_registrations: 1,
            _pd: PhantomData,
        });

        let subject = U256::from(0);
        SubspaceMod::add_balance_to_account(&subject, to_nano(2));

        let origin = get_origin(subject.clone());
        let module_key = U256::from(0);
        assert_err!(
            SubspaceMod::register(
                origin.clone(),
                b"unknown-network".to_vec(),
                b"name".to_vec(),
                b"address".to_vec(),
                to_nano(2),
                module_key,
                None
            ),
            Error::<Test>::NotEnoughBalanceToRegisterSubnet
        );
    });
}

#[test]
fn test_subnet_burn_adjust() {
    new_test_ext().execute_with(|| {
        SubnetBurnConfig::<Test>::set(BurnConfiguration::<Test> {
            min_burn: 2_000_000_000_000,
            max_burn: 100_000_000_000_000,
            adjustment_alpha: (u64::MAX as f32 / 2f32 * 1.2) as u64,
            adjustment_interval: 2_000,
            expected_registrations: 1,
            _pd: PhantomData,
        });

        let burn_configuration = SubnetBurnConfig::<Test>::get();
        assert_eq!(SubnetBurn::<Test>::get(), burn_configuration.min_burn);

        step_block(burn_configuration.adjustment_interval + 1);

        assert_eq!(
            SubnetBurn::<Test>::get(),
            /* 1_600_000_000_000 */ to_nano(2000)
        ); // less than min_burn so it's capped to min_burn

        register_module(0, U256::from(0), to_nano(60)).expect("could not register module");
        step_block(burn_configuration.adjustment_interval + 1);

        assert_eq!(SubnetBurn::<Test>::get(), to_nano(2000));

        register_module(1, U256::from(0), to_nano(60)).expect("could not register module");
        register_module(2, U256::from(0), to_nano(60)).expect("could not register module");
        step_block(burn_configuration.adjustment_interval + 1);

        dbg!(SubnetRegistrationsThisInterval::<Test>::get());

        assert_eq!(SubnetBurn::<Test>::get() / 1_000_000_000, 2_400);
    });
}
