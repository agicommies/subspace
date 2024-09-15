use crate::{
    subnet_consensus::util::{consensus::*, params},
    EmissionError,
};

use crate::Config;
use core::marker::PhantomData;
use frame_support::DebugNoBound;
use pallet_subspace::math::*;
use sp_std::{vec, vec::Vec};

#[derive(DebugNoBound)]
pub struct YumaEpoch<T: Config> {
    subnet_id: u16,

    params: params::ConsensusParams<T>,
    modules: params::FlattenedModules<T::AccountId>,

    _pd: PhantomData<T>,
}

impl<T: Config> YumaEpoch<T> {
    pub fn new(subnet_id: u16, mut params: params::ConsensusParams<T>) -> Self {
        let modules = sp_std::mem::take(&mut params.modules).into();

        Self {
            subnet_id,

            params,
            modules,

            _pd: Default::default(),
        }
    }

    pub fn run(self) -> Result<ConsensusOutput<T>, EmissionError> {
        log::debug!(
            "running yuma for subnet_id {}, will emit {:?} modules and {:?} to founder",
            self.subnet_id,
            self.params.token_emission,
            self.params.founder_emission
        );
        log::trace!("yuma for subnet_id {} parameters: {self:?}", self.subnet_id);

        let (inactive, active) = split_modules_by_activity(
            &self.modules.last_update,
            &self.modules.block_at_registration,
            self.params.activity_cutoff,
            self.params.current_block,
        );

        let mut weights = compute_weights(&self.modules, &self.params)
            .ok_or(EmissionError::Other("weights are broken"))?;

        let stake = StakeVal::unchecked_from_inner(self.modules.stake_normalized.clone());
        log::trace!("final stake: {stake:?}");

        let new_permits: Vec<bool> = if let Some(max) = self.params.max_allowed_validators {
            is_topk(stake.as_ref(), max as usize)
        } else {
            vec![true; stake.as_ref().len()]
        };

        log::trace!("new permis: {new_permits:?}");

        let active_stake = compute_active_stake(&self.modules, &self.params, &inactive, &stake);
        log::trace!("final active stake: {active_stake:?}");

        let ConsensusAndTrust {
            consensus,
            validator_trust,
            preranks,
        } = compute_consensus_and_trust_yuma(
            &self.modules,
            &self.params,
            &mut weights,
            &active_stake,
        );

        let IncentivesAndTrust {
            incentives,
            ranks,
            trust,
        } = compute_incentive_and_trust::<T>(&self.modules, &weights, &active_stake, &preranks);

        let BondsAndDividends {
            ema_bonds,
            dividends,
        } = compute_bonds_and_dividends_yuma(
            &self.params,
            &self.modules,
            &consensus,
            &weights,
            &active_stake,
            &incentives,
        )
        .ok_or(EmissionError::Other("bonds storage is broken"))?;

        process_consensus_output::<T>(
            &self.params,
            &self.modules,
            stake,
            active_stake,
            consensus,
            incentives,
            dividends,
            trust,
            ranks,
            active,
            validator_trust,
            new_permits,
            &ema_bonds,
        )
    }
}
