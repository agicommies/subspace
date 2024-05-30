use crate::{
    Config, DelegatedVotingPower, Error, Event, GovernanceConfig, GovernanceConfiguration, Pallet,
    Percent, Proposals, SubnetGovernanceConfig, SubnetId, UnrewardedProposals,
};
use frame_support::{
    dispatch::DispatchResult, ensure, sp_runtime::traits::IntegerSquareRoot,
    storage::with_storage_layer, traits::ConstU32, BoundedBTreeMap, BoundedBTreeSet, BoundedVec,
    DebugNoBound,
};
use frame_system::ensure_signed;
use pallet_subspace::{
    subnet::SubnetChangeset, voting::VoteMode, DaoTreasuryAddress, Event as SubspaceEvent,
    GlobalParams, Pallet as PalletSubspace, SubnetParams, VoteModeSubnet,
};
use parity_scale_codec::{Decode, Encode, MaxEncodedLen};
use scale_info::TypeInfo;
use sp_std::{collections::btree_map::BTreeMap, vec::Vec};
use substrate_fixed::types::I92F36;

pub type ProposalId = u64;

#[derive(DebugNoBound, TypeInfo, Decode, Encode, MaxEncodedLen)]
#[scale_info(skip_type_params(T))]
pub struct Proposal<T: Config> {
    pub id: ProposalId,
    pub proposer: T::AccountId,
    pub expiration_block: u64,
    pub data: ProposalData<T>,
    pub status: ProposalStatus<T>,
    pub metadata: BoundedVec<u8, ConstU32<256>>,
    pub proposal_cost: u64,
    pub creation_block: u64,
}

impl<T: Config> Proposal<T> {
    /// Whether the proposal is still active.
    #[must_use]
    pub fn is_active(&self) -> bool {
        matches!(self.status, ProposalStatus::Open { .. })
    }

    /// Returns the subnet ID that this proposal impact.s
    #[must_use]
    pub fn subnet_id(&self) -> Option<u16> {
        match &self.data {
            ProposalData::SubnetParams { subnet_id, .. }
            | ProposalData::SubnetCustom { subnet_id, .. } => Some(*subnet_id),
            _ => None,
        }
    }

    /// Marks a proposal as accepted and overrides the storage value.
    pub fn accept(mut self, block_number: u64) -> DispatchResult {
        ensure!(self.is_active(), Error::<T>::ProposalIsFinished);

        self.status = ProposalStatus::Accepted {
            block: block_number,
            stake_for: 0,
            stake_against: 0,
        };

        Proposals::<T>::insert(self.id, &self);
        Pallet::<T>::deposit_event(Event::ProposalAccepted(self.id));

        self.execute_proposal()?;

        Ok(())
    }

    fn execute_proposal(self) -> DispatchResult {
        PalletSubspace::<T>::add_balance_to_account(
            &self.proposer,
            PalletSubspace::<T>::u64_to_balance(self.proposal_cost).unwrap(),
        );

        match self.data {
            ProposalData::GlobalCustom | ProposalData::SubnetCustom { .. } => {
                // No specific action needed for custom proposals
                // The owners will handle the off-chain logic
            }
            ProposalData::GlobalParams(params) => {
                PalletSubspace::<T>::set_global_params(params.clone());
                PalletSubspace::<T>::deposit_event(SubspaceEvent::GlobalParamsUpdated(params));
            }
            ProposalData::SubnetParams { subnet_id, params } => {
                let changeset = SubnetChangeset::<T>::update(subnet_id, params)?;
                changeset.apply(subnet_id)?;
                PalletSubspace::<T>::deposit_event(SubspaceEvent::SubnetParamsUpdated(subnet_id));
            }
            ProposalData::TransferDaoTreasury { account, amount } => {
                PalletSubspace::<T>::remove_balance_from_account(
                    &DaoTreasuryAddress::<T>::get(),
                    PalletSubspace::<T>::u64_to_balance(amount)
                        .ok_or(Error::<T>::InvalidCurrencyConversionValue)?,
                )?;

                let amount = PalletSubspace::<T>::u64_to_balance(amount)
                    .ok_or(Error::<T>::InvalidCurrencyConversionValue)?;
                PalletSubspace::<T>::add_balance_to_account(&account, amount);
            }
        }

        Ok(())
    }

    /// Marks a proposal as refused and overrides the storage value.
    pub fn refuse(mut self, block_number: u64) -> DispatchResult {
        ensure!(self.is_active(), Error::<T>::ProposalIsFinished);

        self.status = ProposalStatus::Refused {
            block: block_number,
            stake_for: 0,
            stake_against: 0,
        };

        Proposals::<T>::insert(self.id, &self);
        Pallet::<T>::deposit_event(Event::ProposalRefused(self.id));

        Ok(())
    }

    /// Marks a proposal as expired and overrides the storage value.
    pub fn expire(mut self, block_number: u64) -> DispatchResult {
        ensure!(self.is_active(), Error::<T>::ProposalIsFinished);
        ensure!(
            block_number >= self.expiration_block,
            Error::<T>::InvalidProposalFinalizationParameters
        );

        self.status = ProposalStatus::Expired;

        Proposals::<T>::insert(self.id, &self);
        Pallet::<T>::deposit_event(Event::ProposalExpired(self.id));

        Ok(())
    }
}

#[derive(Clone, DebugNoBound, TypeInfo, Decode, Encode, MaxEncodedLen, PartialEq, Eq)]
#[scale_info(skip_type_params(T))]
pub enum ProposalStatus<T: Config> {
    Open {
        votes_for: BoundedBTreeSet<T::AccountId, ConstU32<{ u32::MAX }>>,
        votes_against: BoundedBTreeSet<T::AccountId, ConstU32<{ u32::MAX }>>,
    },
    Accepted {
        block: u64,
        stake_for: u64,
        stake_against: u64,
    },
    Refused {
        block: u64,
        stake_for: u64,
        stake_against: u64,
    },
    Expired,
}

#[derive(DebugNoBound, TypeInfo, Decode, Encode, MaxEncodedLen, PartialEq, Eq)]
#[scale_info(skip_type_params(T))]
pub enum ProposalData<T: Config> {
    GlobalCustom,
    GlobalParams(pallet_subspace::GlobalParams<T>),
    SubnetCustom {
        subnet_id: SubnetId,
    },
    SubnetParams {
        subnet_id: SubnetId,
        params: pallet_subspace::SubnetParams<T>,
    },
    TransferDaoTreasury {
        account: T::AccountId,
        amount: u64,
    },
}

impl<T: Config> ProposalData<T> {
    /// The required amount of stake each of the proposal types requires in order to pass.
    #[must_use]
    pub fn required_stake(&self) -> Percent {
        match self {
            Self::GlobalCustom | Self::SubnetCustom { .. } | Self::TransferDaoTreasury { .. } => {
                Percent::from_parts(50)
            }
            Self::GlobalParams(_) | Self::SubnetParams { .. } => Percent::from_parts(40),
        }
    }
}

#[derive(DebugNoBound, TypeInfo, Decode, Encode, MaxEncodedLen, PartialEq, Eq)]
#[scale_info(skip_type_params(T))]
pub struct UnrewardedProposal<T: Config> {
    pub subnet_id: Option<SubnetId>,
    pub block: u64,
    pub votes_for: BoundedBTreeMap<T::AccountId, u64, ConstU32<{ u32::MAX }>>,
    pub votes_against: BoundedBTreeMap<T::AccountId, u64, ConstU32<{ u32::MAX }>>,
}

impl<T: Config> Pallet<T> {
    fn get_next_proposal_id() -> u64 {
        match Proposals::<T>::iter_keys().max() {
            Some(id) => id + 1,
            None => 0,
        }
    }

    pub fn add_proposal(
        key: T::AccountId,
        metadata: BoundedVec<u8, ConstU32<256>>,
        data: ProposalData<T>,
    ) -> DispatchResult {
        let GovernanceConfiguration {
            proposal_cost,
            expiration,
            ..
        } = GovernanceConfig::<T>::get();

        ensure!(
            PalletSubspace::<T>::has_enough_balance(&key, proposal_cost),
            Error::<T>::NotEnoughBalanceToPropose
        );

        let Some(removed_balance_as_currency) = PalletSubspace::<T>::u64_to_balance(proposal_cost)
        else {
            return Err(Error::<T>::InvalidCurrencyConversionValue.into());
        };

        let proposal_id = Self::get_next_proposal_id();
        let current_block = PalletSubspace::<T>::get_current_block_number();
        let expiration_block = current_block + expiration as u64;

        // TODO: extract rounding function
        let expiration_block = if expiration_block % 100 == 0 {
            expiration_block
        } else {
            expiration_block + 100 - (expiration_block % 100)
        };

        let proposal = Proposal {
            id: proposal_id,
            proposer: key.clone(),
            expiration_block,
            data,
            status: ProposalStatus::Open {
                votes_for: BoundedBTreeSet::new(),
                votes_against: BoundedBTreeSet::new(),
            },
            proposal_cost,
            creation_block: current_block,
            metadata,
        };

        // Burn the proposal cost from the proposer's balance
        PalletSubspace::<T>::remove_balance_from_account(&key, removed_balance_as_currency)?;

        Proposals::<T>::insert(proposal_id, proposal);

        Self::deposit_event(Event::<T>::ProposalCreated(proposal_id));
        Ok(())
    }

    pub fn do_add_custom_proposal(origin: T::RuntimeOrigin, data: Vec<u8>) -> DispatchResult {
        let key = ensure_signed(origin)?;
        ensure!(!data.is_empty(), Error::<T>::ProposalDataTooSmall);
        ensure!(data.len() <= 256, Error::<T>::ProposalDataTooLarge);
        sp_std::str::from_utf8(&data).map_err(|_| Error::<T>::InvalidProposalData)?;

        let proposal_data = ProposalData::GlobalCustom;
        Self::add_proposal(key, BoundedVec::truncate_from(data), proposal_data)
    }

    pub fn do_add_custom_subnet_proposal(
        origin: T::RuntimeOrigin,
        netuid: u16,
        data: Vec<u8>,
    ) -> DispatchResult {
        let key = ensure_signed(origin)?;
        ensure!(!data.is_empty(), Error::<T>::ProposalDataTooSmall);
        ensure!(data.len() <= 256, Error::<T>::ProposalDataTooLarge);
        sp_std::str::from_utf8(&data).map_err(|_| Error::<T>::InvalidProposalData)?;

        let proposal_data = ProposalData::SubnetCustom { subnet_id: netuid };
        Self::add_proposal(key, BoundedVec::truncate_from(data), proposal_data)
    }

    pub fn do_add_transfer_dao_treasury_proposal(
        origin: T::RuntimeOrigin,
        data: Vec<u8>,
        value: u64,
        dest: T::AccountId,
    ) -> DispatchResult {
        let key = ensure_signed(origin)?;
        ensure!(!data.is_empty(), Error::<T>::ProposalDataTooSmall);
        ensure!(data.len() <= 256, Error::<T>::ProposalDataTooLarge);
        ensure!(
            PalletSubspace::<T>::has_enough_balance(&DaoTreasuryAddress::<T>::get(), value),
            Error::<T>::InsufficientDaoTreasuryFunds
        );
        sp_std::str::from_utf8(&data).map_err(|_| Error::<T>::InvalidProposalData)?;

        let proposal_data = ProposalData::TransferDaoTreasury {
            amount: value,
            account: dest,
        };
        Self::add_proposal(key, BoundedVec::truncate_from(data), proposal_data)
    }

    pub fn do_add_global_proposal(
        origin: T::RuntimeOrigin,
        data: Vec<u8>,
        params: GlobalParams<T>,
    ) -> DispatchResult {
        let key = ensure_signed(origin)?;

        ensure!(!data.is_empty(), Error::<T>::ProposalDataTooSmall);
        ensure!(data.len() <= 256, Error::<T>::ProposalDataTooLarge);
        PalletSubspace::check_global_params(&params)?;

        let proposal_data = ProposalData::GlobalParams(params);
        Self::add_proposal(key, BoundedVec::truncate_from(data), proposal_data)
    }

    pub fn do_add_subnet_proposal(
        origin: T::RuntimeOrigin,
        netuid: u16,
        data: Vec<u8>,
        params: SubnetParams<T>,
    ) -> DispatchResult {
        let key = ensure_signed(origin)?;

        let vote_mode = VoteModeSubnet::<T>::get(netuid);
        ensure!(vote_mode == VoteMode::Vote, Error::<T>::NotVoteMode);

        ensure!(!data.is_empty(), Error::<T>::ProposalDataTooSmall);
        ensure!(data.len() <= 256, Error::<T>::ProposalDataTooLarge);

        SubnetChangeset::<T>::update(netuid, params.clone())?;
        let proposal_data = ProposalData::SubnetParams {
            subnet_id: netuid,
            params,
        };
        Self::add_proposal(key, BoundedVec::truncate_from(data), proposal_data)
    }
}

pub fn tick_proposals<T: Config>(block_number: u64) {
    let delegating = DelegatedVotingPower::<T>::iter().fold(
        BTreeMap::<_, u64>::new(),
        |mut acc, (delegated, subnet_id, delegators)| {
            for delegator in delegators {
                let Ok(stakes) = pallet_subspace::StakeTo::<T>::try_get(subnet_id, &delegator)
                else {
                    continue;
                };

                if let Some(stake) = stakes.get(&delegated) {
                    let key = acc.entry((delegator.clone(), subnet_id)).or_default();
                    *key = key.saturating_add(*stake);
                }
            }

            acc
        },
    );

    for (id, proposal) in Proposals::<T>::iter().filter(|(_, p)| p.is_active()) {
        let res = with_storage_layer(|| tick_proposal(&delegating, block_number, proposal));
        if let Err(err) = res {
            log::error!("failed to tick proposal {id}: {err:?}, skipping...");
        }
    }
}

fn tick_proposal<T: Config>(
    delegating: &BTreeMap<(T::AccountId, u16), u64>,
    block_number: u64,
    proposal: Proposal<T>,
) -> DispatchResult {
    let subnet_id = proposal.subnet_id();

    let ProposalStatus::Open {
        votes_for,
        votes_against,
    } = &proposal.status
    else {
        return Err(Error::<T>::ProposalIsFinished.into());
    };

    let votes_for: Vec<(T::AccountId, u64)> = votes_for
        .iter()
        .cloned()
        .map(|id| {
            let stake = calc_stake::<T>(delegating, &id, subnet_id);
            (id, stake)
        })
        .collect();
    let votes_against: Vec<(T::AccountId, u64)> = votes_against
        .iter()
        .cloned()
        .map(|id| {
            let stake = calc_stake::<T>(delegating, &id, subnet_id);
            (id, stake)
        })
        .collect();

    let stake_for_sum: u64 = votes_for.iter().map(|(_, stake)| stake).sum();
    let stake_against_sum: u64 = votes_for.iter().map(|(_, stake)| stake).sum();

    let total_stake = stake_for_sum + stake_against_sum;
    let minimal_stake_to_execute =
        PalletSubspace::<T>::get_minimal_stake_to_execute_with_percentage(
            proposal.data.required_stake(),
            subnet_id,
        );

    let mut reward_votes_for = BoundedBTreeMap::new();
    for (key, value) in votes_for {
        reward_votes_for.try_insert(key, value).expect("this wont exceed u32::MAX");
    }

    let mut reward_votes_against: BoundedBTreeMap<T::AccountId, u64, ConstU32<{ u32::MAX }>> =
        BoundedBTreeMap::new();
    for (key, value) in votes_against {
        reward_votes_against
            .try_insert(key, value)
            .expect("this probably wont exceed u32::MAX");
    }

    if total_stake >= minimal_stake_to_execute {
        UnrewardedProposals::<T>::insert(
            proposal.id,
            UnrewardedProposal::<T> {
                subnet_id: proposal.subnet_id(),
                block: block_number,
                votes_for: reward_votes_for,
                votes_against: reward_votes_against,
            },
        );

        if stake_against_sum > stake_for_sum {
            proposal.refuse(block_number)?;
        } else {
            proposal.accept(block_number)?;
        }

        Ok(())
    } else if block_number >= proposal.expiration_block {
        UnrewardedProposals::<T>::insert(
            proposal.id,
            UnrewardedProposal::<T> {
                subnet_id: proposal.subnet_id(),
                block: block_number,
                votes_for: reward_votes_for,
                votes_against: reward_votes_against,
            },
        );

        proposal.expire(block_number)?;

        Ok(())
    } else {
        Ok(())
    }
}

pub fn tick_proposal_rewards<T: Config>(block_number: u64) {
    let mut to_tick: Vec<(Option<u16>, GovernanceConfiguration<T>)> =
        pallet_subspace::N::<T>::iter_keys()
            .map(|subnet_id| (Some(subnet_id), SubnetGovernanceConfig::<T>::get(subnet_id)))
            .collect();
    to_tick.push((None, GovernanceConfig::<T>::get()));

    to_tick.into_iter().for_each(|(subnet_id, governance_config)| {
        execute_proposal_rewards(block_number, subnet_id, governance_config);
    });
}

fn calc_stake<T: Config>(
    delegating: &BTreeMap<(T::AccountId, u16), u64>,
    voter: &T::AccountId,
    subnet_id: Option<SubnetId>,
) -> u64 {
    if let Some(subnet_id) = subnet_id {
        let own_stake: u64 =
            pallet_subspace::StakeTo::<T>::get(subnet_id, voter).into_values().sum();
        let voter_delegated_stake =
            delegating.get(&(voter.clone(), subnet_id)).copied().unwrap_or_default();
        let own_stake = own_stake.saturating_sub(voter_delegated_stake);
        let delegated_stake = DelegatedVotingPower::<T>::get(voter, subnet_id)
            .iter()
            .map(|delegator| PalletSubspace::<T>::get_stake_to_module(subnet_id, delegator, voter))
            .sum::<u64>();
        own_stake + delegated_stake
    } else {
        let mut own_stake: u64 = 0;
        let mut delegated_stake: u64 = 0;

        for subnet_id in pallet_subspace::N::<T>::iter_keys() {
            let stake: u64 = pallet_subspace::StakeTo::<T>::try_get(subnet_id, voter)
                .map(|v| v.into_values().sum())
                .unwrap_or_default();
            let voter_delegated_stake =
                delegating.get(&(voter.clone(), subnet_id)).copied().unwrap_or_default();
            own_stake = own_stake.saturating_add(stake.saturating_sub(voter_delegated_stake));

            delegated_stake = delegated_stake.saturating_add(
                DelegatedVotingPower::<T>::get(voter, subnet_id)
                    .iter()
                    .filter_map(|delegator| {
                        pallet_subspace::StakeTo::<T>::try_get(subnet_id, delegator)
                            .ok()?
                            .get(voter)
                            .copied()
                    })
                    .sum::<u64>(),
            );
        }

        own_stake + delegated_stake
    }
}

pub fn execute_proposal_rewards<T: crate::Config>(
    block_number: u64,
    subnet_id: Option<u16>,
    governance_config: GovernanceConfiguration<T>,
) {
    let mut n: u16 = 0;
    for (proposal_id, unrewarded_proposal) in UnrewardedProposals::<T>::iter() {
        if subnet_id != unrewarded_proposal.subnet_id {
            continue;
        }

        if unrewarded_proposal.block < block_number - governance_config.proposal_reward_interval {
            continue;
        }

        execute_proposal_reward(proposal_id, unrewarded_proposal, &governance_config, n);
        UnrewardedProposals::<T>::remove(proposal_id);
        n += 1;
    }
}

fn execute_proposal_reward<T: crate::Config>(
    proposal_id: ProposalId,
    unrewarded_proposal: UnrewardedProposal<T>,
    governance_config: &GovernanceConfiguration<T>,
    n: u16,
) {
    let treasury_address = DaoTreasuryAddress::<T>::get();
    let treasury_balance = PalletSubspace::<T>::get_balance(&treasury_address);
    let treasury_balance = I92F36::from_num(PalletSubspace::<T>::balance_to_u64(treasury_balance));

    let allocation_percentage = governance_config.proposal_reward_treasury_allocation;
    let max_allocation =
        I92F36::from_num(governance_config.max_proposal_reward_treasury_allocation);

    let mut allocation = (treasury_balance / allocation_percentage).min(max_allocation);
    if n > 0 {
        let mut base = I92F36::from_num(1.5);
        let mut result = I92F36::from_num(1);
        let mut remaining = n;

        while remaining > 0 {
            if remaining % 2 == 1 {
                result *= base;
            }
            base = base * base;
            remaining /= 2;
        }

        allocation /= result;
    }

    let mut accs_with_stake: Vec<(T::AccountId, u64)> = Vec::new();
    for (acc_id, stake) in unrewarded_proposal.votes_for {
        accs_with_stake.push((acc_id, stake));
    }
    for (acc_id, stake) in unrewarded_proposal.votes_against {
        accs_with_stake.push((acc_id, stake));
    }

    let reward_by_account = distribute_rewards::<T>(accs_with_stake, allocation);

    for (acc_id, reward) in reward_by_account {
        if let Err(err) =
            PalletSubspace::<T>::transfer_balance_to_account(&treasury_address, &acc_id, reward)
        {
            log::error!(
                "could not transfer #{}'s voting reward of {} tokens for account {:?}: {err:#?}",
                proposal_id,
                reward,
                acc_id
            );
        }
    }
}

fn distribute_rewards<T: crate::Config>(
    accs_with_stake: Vec<(T::AccountId, u64)>,
    allocation: I92F36,
) -> Vec<(T::AccountId, u64)> {
    let accs_with_sqrt_stake: Vec<_> = accs_with_stake
        .into_iter()
        .map(|(acc_id, stake)| (acc_id, stake.integer_sqrt()))
        .collect();

    let total_stake =
        I92F36::from_num(accs_with_sqrt_stake.iter().map(|(_, stake)| stake).sum::<u64>());

    accs_with_sqrt_stake
        .into_iter()
        .map(|(acc_id, stake)| {
            let percentage = I92F36::from_num(stake) / total_stake;
            (acc_id, (allocation * percentage).to_num())
        })
        .collect()
}
