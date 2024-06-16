use crate::{
    math::*, Config, DaoTreasuryAddress, Dividends, Emission, Founder, GlobalParams, Incentive,
    IncentiveRatio, LastUpdate, MaxAllowedValidators, MaxWeightAge, Pallet, Stake, SubnetParams,
    TotalStake, Trust, TrustRatio, ValidatorPermits, Vec, Weights, N,
};
use core::marker::PhantomData;
use sp_arithmetic::per_things::Percent;
use sp_std::vec;
use substrate_fixed::types::{I32F32, I64F64};

use super::{yuma::AccountKey, EmissionError};

pub struct LinearEpoch<T: Config> {
    module_count: u16,
    netuid: u16,

    founder_key: T::AccountId,
    founder_emission: u64,
    to_be_emitted: u64,

    current_block: u64,
    activity_cutoff: u64,
    last_update: Vec<u64>,

    global_params: GlobalParams<T>,
    subnet_params: SubnetParams<T>,

    _pd: PhantomData<T>,
}

impl<T: Config> LinearEpoch<T> {
    pub fn new(netuid: u16, to_be_emitted: u64) -> Self {
        let founder_key = Founder::<T>::get(netuid);
        let (to_be_emitted, founder_emission) =
            Pallet::<T>::calculate_founder_emission(netuid, to_be_emitted);

        let global_params = Pallet::<T>::global_params();
        let subnet_params = Pallet::<T>::subnet_params(netuid);

        Self {
            module_count: N::<T>::get(netuid),
            netuid,

            founder_key,
            founder_emission,
            to_be_emitted,

            current_block: Pallet::<T>::get_current_block_number(),
            activity_cutoff: MaxWeightAge::<T>::get(netuid),
            last_update: LastUpdate::<T>::get(netuid),

            global_params,
            subnet_params,

            _pd: Default::default(),
        }
    }

    /// This function acts as the main function of the entire blockchain reward distribution.
    /// It calculates the dividends, the incentive, the weights, the bonds,
    /// the trust and the emission for the epoch.
    pub fn run(self) -> Result<(), EmissionError> {
        if self.module_count == 0 {
            return Ok(());
        }

        // STAKE
        let uid_key_tuples: Vec<(u16, T::AccountId)> = Pallet::<T>::get_uid_key_tuples(self.netuid);
        let total_stake_u64: u64 = Pallet::<T>::get_total_subnet_stake(self.netuid).max(1);

        let stake_u64: Vec<u64> =
            uid_key_tuples.iter().map(|(_, key)| Stake::<T>::get(key)).collect();

        let stake_f64: Vec<I64F64> = stake_u64
            .iter()
            .map(|x| I64F64::from_num(*x) / I64F64::from_num(total_stake_u64))
            .collect();

        let mut stake: Vec<I32F32> = stake_f64.iter().map(|x| I32F32::from_num(*x)).collect();

        // Normalize stake.
        inplace_normalize(&mut stake);

        // WEIGHTS
        let weights: Vec<Vec<(u16, I32F32)>> = Self::process_weights(
            self.netuid,
            self.module_count,
            &self.global_params,
            &self.subnet_params,
            self.current_block,
            &stake_f64,
            total_stake_u64,
            self.last_update,
        );

        // INCENTIVE
        // see if this shit needs to be mut
        let mut incentive: Vec<I32F32> =
            Self::compute_incentive(&weights, &stake, &uid_key_tuples, self.module_count);

        // TRUST
        // trust that acts as a multiplier for the incentive
        let trust_ratio: u16 = TrustRatio::<T>::get(self.netuid);
        if trust_ratio > 0 {
            let trust_share: I32F32 = I32F32::from_num(trust_ratio) / I32F32::from_num(100);
            let incentive_share: I32F32 = I32F32::from_num(1.0).saturating_sub(trust_share);
            let trust =
                Self::compute_trust(&weights, &stake, &self.subnet_params, self.module_count);

            incentive = incentive
                .iter()
                .zip(trust.iter())
                .map(|(inc, tru)| (inc * incentive_share) + (tru * trust_share))
                .collect();

            // save the trust into the trust vector
            Trust::<T>::insert(
                self.netuid,
                trust.iter().map(|xi| fixed_proportion_to_u16(*xi)).collect::<Vec<u16>>(),
            );
        }

        // store the incentive
        let cloned_incentive: Vec<u16> =
            incentive.iter().map(|xi| fixed_proportion_to_u16(*xi)).collect::<Vec<u16>>();
        Incentive::<T>::insert(self.netuid, cloned_incentive);

        //  BONDS
        let bonds: Vec<Vec<(u16, I32F32)>> = Self::compute_bonds_delta(&weights, &stake);

        // DIVIDENDS
        let (fixed_dividends, dividends) =
            Self::compute_dividends(&bonds, &incentive, &uid_key_tuples);
        Dividends::<T>::insert(self.netuid, fixed_dividends);

        // EMISSION
        Self::process_emission(
            &incentive,
            &dividends,
            self.to_be_emitted,
            self.netuid,
            self.founder_emission,
            &self.founder_key,
            &uid_key_tuples,
        );

        Ok(())
    }

    fn calculate_emission_ratios(
        incentive: &[I32F32],
        dividends: &[I32F32],
        token_emission: u64,
        netuid: u16,
    ) -> (Vec<I64F64>, Vec<I64F64>) {
        let incentive_ratio: I64F64 =
            I64F64::from_num(IncentiveRatio::<T>::get(netuid) as u64) / I64F64::from_num(100);
        let dividend_ratio: I64F64 = I64F64::from_num(1.0) - incentive_ratio;

        let incentive_emission_float: Vec<I64F64> = incentive
            .iter()
            .map(|&x| I64F64::from_num(x) * I64F64::from_num(token_emission) * incentive_ratio)
            .collect();
        let dividends_emission_float: Vec<I64F64> = dividends
            .iter()
            .map(|&x| I64F64::from_num(x) * I64F64::from_num(token_emission) * dividend_ratio)
            .collect();

        (incentive_emission_float, dividends_emission_float)
    }

    fn calculate_emissions(
        incentive_emission_float: &[I64F64],
        dividends_emission_float: &[I64F64],
        founder_emission: u64,
        netuid: u16,
        founder_key: &T::AccountId,
        uid_key_tuples: &[(u16, T::AccountId)],
    ) -> Vec<u64> {
        let n = incentive_emission_float.len();
        let mut incentive_emission: Vec<u64> =
            incentive_emission_float.iter().map(|e| e.to_num::<u64>()).collect();
        let dividends_emission: Vec<u64> =
            dividends_emission_float.iter().map(|e| e.to_num::<u64>()).collect();

        if netuid != 0 {
            let founder_uid = Pallet::<T>::get_uid_for_key(netuid, founder_key);
            incentive_emission[founder_uid as usize] =
                incentive_emission[founder_uid as usize].saturating_add(founder_emission);
        }

        let mut emission: Vec<u64> = vec![0; n];
        let mut emitted = 0u64;

        for (module_uid, module_key) in uid_key_tuples.iter() {
            let owner_emission_incentive: u64 = incentive_emission[*module_uid as usize];
            let mut owner_dividends_emission: u64 = dividends_emission[*module_uid as usize];

            emission[*module_uid as usize] = owner_emission_incentive + owner_dividends_emission;

            if owner_dividends_emission > 0 {
                let ownership_vector: Vec<(T::AccountId, I64F64)> =
                    Pallet::<T>::get_ownership_ratios(netuid, module_key);

                let delegation_fee = Pallet::<T>::get_delegation_fee(netuid, module_key);

                let total_owner_dividends_emission: u64 = owner_dividends_emission;
                for (delegate_key, delegate_ratio) in ownership_vector.iter() {
                    if delegate_key == module_key {
                        continue;
                    }

                    let dividends_from_delegate: u64 =
                        (I64F64::from_num(total_owner_dividends_emission) * *delegate_ratio)
                            .to_num::<u64>();
                    let to_module: u64 = delegation_fee.mul_floor(dividends_from_delegate);
                    let to_delegate: u64 = dividends_from_delegate.saturating_sub(to_module);
                    Pallet::<T>::increase_stake(delegate_key, module_key, to_delegate);
                    emitted = emitted.saturating_add(to_delegate);
                    owner_dividends_emission = owner_dividends_emission.saturating_sub(to_delegate);
                }
            }

            let owner_emission: u64 = owner_emission_incentive + owner_dividends_emission;
            if owner_emission > 0 {
                let profit_share_emissions: Vec<(T::AccountId, u64)> =
                    Pallet::<T>::get_profit_share_emissions(module_key, owner_emission);

                if !profit_share_emissions.is_empty() {
                    for (profit_share_key, profit_share_emission) in profit_share_emissions.iter() {
                        Pallet::<T>::increase_stake(
                            profit_share_key,
                            module_key,
                            *profit_share_emission,
                        );
                        emitted = emitted.saturating_add(*profit_share_emission);
                    }
                } else {
                    Pallet::<T>::increase_stake(module_key, module_key, owner_emission);
                    emitted = emitted.saturating_add(owner_emission);
                }
            }
        }

        if netuid == 0 && founder_emission > 0 {
            // Update global treasure
            Pallet::<T>::add_balance_to_account(
                &DaoTreasuryAddress::<T>::get(),
                Pallet::<T>::u64_to_balance(founder_emission).unwrap_or_default(),
            );
        }
        emission
    }

    fn process_emission(
        incentive: &[I32F32],
        dividends: &[I32F32],
        to_be_emitted: u64,
        netuid: u16,
        founder_emission: u64,
        founder_key: &T::AccountId,
        uid_key_tuples: &[(u16, T::AccountId)],
    ) {
        let (incentive_emission_float, dividends_emission_float) =
            Self::calculate_emission_ratios(incentive, dividends, to_be_emitted, netuid);

        let emission = Self::calculate_emissions(
            &incentive_emission_float,
            &dividends_emission_float,
            founder_emission,
            netuid,
            founder_key,
            uid_key_tuples,
        );

        Emission::<T>::insert(netuid, emission);
    }

    fn compute_dividends(
        bonds: &[Vec<(u16, I32F32)>],
        incentive: &[I32F32],
        uid_key_tuples: &[(u16, T::AccountId)],
    ) -> (Vec<u16>, Vec<I32F32>) {
        let n = incentive.len();
        let mut dividends: Vec<I32F32> = vec![I32F32::from_num(0.0); n];

        for (i, sparse_row) in bonds.iter().enumerate() {
            for (j, value) in sparse_row.iter() {
                dividends[i] += incentive[*j as usize] * *value;
            }
        }

        if dividends.iter().all(|&x| x == I32F32::from_num(0.0)) {
            for (uid_i, _) in uid_key_tuples.iter() {
                dividends[*uid_i as usize] = I32F32::from_num(1.0);
            }
        }

        inplace_normalize(&mut dividends);

        let fixed_dividends: Vec<u16> =
            dividends.iter().map(|xi| fixed_proportion_to_u16(*xi)).collect();

        (fixed_dividends, dividends)
    }

    fn compute_bonds_delta(
        weights: &[Vec<(u16, I32F32)>],
        stake: &[I32F32],
    ) -> Vec<Vec<(u16, I32F32)>> {
        let n = weights.len();
        let mut bonds: Vec<Vec<(u16, I32F32)>> = weights.to_vec();
        let mut col_sum: Vec<I32F32> = vec![I32F32::from_num(0.0); n];

        for (i, sparse_row) in bonds.iter_mut().enumerate() {
            for (j, value) in sparse_row.iter_mut() {
                *value *= stake[i];
                col_sum[*j as usize] += *value;
            }
        }

        for sparse_row in bonds.iter_mut() {
            for (j, value) in sparse_row.iter_mut() {
                if col_sum[*j as usize] > I32F32::from_num(0.0) {
                    *value /= col_sum[*j as usize];
                }
            }
        }

        bonds
    }

    fn compute_trust(
        weights: &[Vec<(u16, I32F32)>],
        stake: &[I32F32],
        subnet_params: &SubnetParams<T>,
        n: u16,
    ) -> Vec<I32F32> {
        let mut trust = vec![I32F32::from_num(0.0); n as usize];
        for (i, weights_i) in weights.iter().enumerate() {
            for (j, weight_ij) in weights_i.iter() {
                if *weight_ij > 0 && stake[i] > I32F32::from_num(subnet_params.min_stake) {
                    trust[*j as usize] += I32F32::from_num(1.0);
                }
            }
        }
        inplace_normalize(&mut trust);
        trust
    }

    fn compute_incentive(
        weights: &[Vec<(u16, I32F32)>],
        stake: &[I32F32],
        uid_key_tuples: &[(u16, T::AccountId)],
        n: u16,
    ) -> Vec<I32F32> {
        let mut incentive: Vec<I32F32> = vec![I32F32::from_num(0.0); n as usize];

        for (i, sparse_row) in weights.iter().enumerate() {
            for (j, value) in sparse_row.iter() {
                incentive[*j as usize] += stake[i] * value;
            }
        }

        if is_zero(&incentive) {
            for (uid_i, _key) in uid_key_tuples.iter() {
                incentive[*uid_i as usize] = I32F32::from_num(1.0);
            }
        }

        inplace_normalize(&mut incentive);
        incentive
    }

    fn get_current_weight_age(last_update_vector: &[u64], current_block: u64, uid_i: u16) -> u64 {
        current_block.saturating_sub(last_update_vector[uid_i as usize])
    }

    #[allow(clippy::too_many_arguments)]
    fn check_weight_validity(
        weight_age: u64,
        subnet_params: &SubnetParams<T>,
        weights_i: &[(u16, u16)],
        stake_f64: &[I64F64],
        total_stake_u64: u64,
        min_weight_stake_f64: I64F64,
        n: u16,
        uid_i: u16,
    ) -> (bool, Vec<(u16, u16)>) {
        let mut valid_weights = Vec::new();

        if weight_age > subnet_params.max_weight_age
            || weights_i.len() < subnet_params.min_allowed_weights as usize
        {
            return (true, valid_weights);
        }

        for (pos, (uid_j, weight_ij)) in weights_i.iter().enumerate() {
            if (pos as u16) > subnet_params.max_allowed_weights || *uid_j >= n {
                return (true, valid_weights);
            }

            let weight_f64 = I64F64::from_num(*weight_ij) / I64F64::from_num(u16::MAX);
            let weight_stake =
                (stake_f64[uid_i as usize] * weight_f64) * I64F64::from_num(total_stake_u64);

            if weight_stake > min_weight_stake_f64 {
                valid_weights.push((*uid_j, *weight_ij));
            } else {
                return (true, valid_weights);
            }
        }

        (false, valid_weights)
    }

    fn process_weights(
        netuid: u16,
        n: u16,
        global_params: &GlobalParams<T>,
        subnet_params: &SubnetParams<T>,
        current_block: u64,
        stake_f64: &[I64F64],
        total_stake_u64: u64,
        last_update: Vec<u64>,
    ) -> Vec<Vec<(u16, I32F32)>> {
        let min_weight_stake_f64 = I64F64::from_num(global_params.min_weight_stake);
        let mut weights: Vec<Vec<(u16, u16)>> = vec![vec![]; n as usize];

        for (uid_i, weights_i) in Weights::<T>::iter_prefix(netuid) {
            let weight_age = Self::get_current_weight_age(&last_update, current_block, uid_i);
            let (weight_changed, valid_weights) = Self::check_weight_validity(
                weight_age,
                subnet_params,
                &weights_i,
                stake_f64,
                total_stake_u64,
                min_weight_stake_f64,
                n,
                uid_i,
            );

            weights[uid_i as usize] = valid_weights;
            if weight_changed {
                <Weights<T>>::insert(netuid, uid_i, weights[uid_i as usize].clone());
            }
        }

        let mut weights: Vec<Vec<(u16, I32F32)>> = weights
            .iter()
            .map(|x| {
                x.iter().map(|(uid, weight)| (*uid, u16_proportion_to_fixed(*weight))).collect()
            })
            .collect();

        weights = mask_diag_sparse(&weights);
        inplace_row_normalize_sparse(&mut weights);

        weights
    }
}
