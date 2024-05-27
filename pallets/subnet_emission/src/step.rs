use super::*;
use frame_support::storage::with_storage_layer;
use pallet_subspace::{SetWeightCallsPerEpoch, Tempo};

use pallet_subspace::subnet_consensus::{linear, yuma};

// Handles the whole emission distribution of the blockchain

// TODO: make sure that the proposals are ticked correctly
impl<T: Config> Pallet<T> {
    pub fn block_step(block_number: u64, emission_per_block: u64) {
        log::debug!("stepping block {block_number:?}");

        // Calculate subnet emission
        let subnets_emission_distribution = Self::get_subnet_pricing(emission_per_block);

        for (netuid, tempo) in Tempo::<T>::iter() {
            let new_queued_emission = subnets_emission_distribution.get(&netuid).unwrap_or(&0);

            let emission_to_drain = PendingEmission::<T>::mutate(netuid, |queued: &mut u64| {
                *queued += new_queued_emission;
                *queued
            });
            log::trace!("subnet {netuid} total pending emission: {emission_to_drain}, increased {new_queued_emission}");

            if Self::blocks_until_next_epoch(netuid, tempo, block_number) > 0 {
                continue;
            }

            log::trace!("running epoch for subnet {netuid}");

            // Clearing `set_weight` rate limiter values.
            let _ = SetWeightCallsPerEpoch::<T>::clear_prefix(netuid, u32::MAX, None);

            if PendingEmission::<T>::get(netuid) > 0 {
                let res = with_storage_layer(|| {
                    if netuid == 0 {
                        // TODO.
                        // return result here, copy the layout of yuma consensus
                        linear::LinearEpoch::<T>::linear_epoch(netuid, emission_to_drain);
                        Ok(())
                    } else {
                        match yuma::YumaEpoch::<T>::new(netuid, emission_to_drain).run() {
                            Ok(_) => Ok(()),
                            Err(err) => {
                                log::error!(
                                    "Failed to run yuma consensus algorithm: {err:?}, skipping this block. \
                                    {emission_to_drain} tokens will be emitted on the next epoch."
                                );
                                Err("yuma failed")
                            }
                        }
                    }
                });

                match res {
                    Ok(()) => {
                        PendingEmission::<T>::insert(netuid, 0);
                        Self::deposit_event(Event::<T>::EpochFinished(netuid));
                    }
                    Err(_) => {
                        return;
                    }
                }
            }
        }
    }

    fn blocks_until_next_epoch(netuid: u16, tempo: u16, block_number: u64) -> u64 {
        // in this case network never runs
        if tempo == 0 {
            return 1000;
        }
        (block_number + netuid as u64) % (tempo as u64)
    }
}
