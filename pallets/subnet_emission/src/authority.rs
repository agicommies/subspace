use core::ops::Index;

use pallet_subspace::UseWeightsEncrytyption;

use super::*;

#[derive(Clone, Encode, Decode, TypeInfo)]
pub struct AuthorityNodeInfo {
    public_key: PublicKey,
    last_keep_alive: u64,
}

#[derive(Clone, Encode, Decode, TypeInfo)]
pub struct SubnetEncryptionInfo {
    pub authority_id: u16,
    pub authority_public_key: PublicKey,
    pub block_assigned: u64,
}

impl<T: Config> Pallet<T> {
    pub fn distribute_subnets_to_nodes(block: u64) {
        let authority_node_count = AuthorityNodes::<T>::get().len();
        if authority_node_count < 1 {
            log::warn!("no authority nodes found");
            return;
        }

        for netuid in pallet_subspace::N::<T>::iter_keys() {
            if !UseWeightsEncrytyption::<T>::get(netuid) {
                continue;
            }

            let data = SubnetAuthorityData::<T>::get(netuid);
            if data.is_some_and(|_data| true /* TODO: check if shouldn't rotate */) {
                return;
            }

            let mut current = AuthorityNodeCursor::<T>::get();
            if current as usize >= authority_node_count {
                current = 0;
            }

            let opt = AuthorityNodes::<T>::get().get(current as usize).cloned();
            let Some(node_info) = opt else {
                log::error!("internal error");
                continue;
            };

            SubnetAuthorityData::<T>::set(
                netuid,
                Some(SubnetEncryptionInfo {
                    authority_id: current,
                    authority_public_key: node_info.public_key,
                    block_assigned: block,
                }),
            );

            AuthorityNodeCursor::<T>::set(current + 1);
        }
    }

    pub fn do_handle_decrypted_weights(netuid: u16, weights: Vec<(u16, Vec<(u16, u16)>)>) {
        let Some(info) = SubnetAuthorityData::<T>::get(netuid) else {
            // should not happen, maybe log
            return;
        };

        for (uid, weights) in &weights {
            let Some(hash) = EncryptedWeightHashes::<T>::get(netuid, uid) else {
                // Something went wrong
                return;
            };

            let Some(hashed) = testthing::offworker::hash_weight(weights.clone()) else {
                // Something went wrong
                return;
            };

            if hash != hashed {
                // incorrect hash
                return;
            }
        }

        // TODO: joao  check weight hashes

        // TODO do stuff with decrypted weights

        let block_number = pallet_subspace::Pallet::<T>::get_current_block_number();
        if block_number - info.block_assigned < 100 {
            // TODO check this number
            return;
        }

        let mut current = AuthorityNodeCursor::<T>::get();
        if current as usize >= AuthorityNodes::<T>::get().len() {
            current = 0;
        }

        let Some(new_node) = AuthorityNodes::<T>::get().get(current as usize).cloned() else {
            // shouldn't happen, maybe log
            return;
        };

        SubnetAuthorityData::<T>::set(
            netuid,
            Some(SubnetEncryptionInfo {
                authority_id: current,
                authority_public_key: new_node.public_key,
                block_assigned: block_number,
            }),
        );
    }

    /// Executes consensus for the oldest set of parameters in the given netuid.
    /// Currently only supports Yuma consensus.
    ///
    /// This function:
    /// 1. Retrieves the oldest ConsensusParameters for the specified netuid
    /// 2. Executes the Yuma consensus using these parameters
    /// 3. Applies the consensus results
    /// 4. Deletes the processed ConsensusParameters from storage
    ///
    /// Parameters:
    /// - netuid: The network ID
    /// - weights: The decrypted weights to be used in the consensus
    ///
    /// Returns:
    /// - Ok(()) if successful
    /// - Err with a descriptive message if an error occurs
    pub fn execute_decrypted_weights(
        netuid: u16,
        weights: Vec<(u16, Vec<(u16, u16)>)>,
    ) -> Result<(), &'static str> {
        // Check if the given netuid is running Yuma consensus
        let consensus_type = SubnetConsensusType::<T>::get(netuid).ok_or("Invalid network ID")?;

        if consensus_type != SubnetConsensus::Yuma {
            return Err("Unsupported consensus type");
        }

        // Retrieve the oldest ConsensusParameters
        let (oldest_epoch, oldest_params) = ConsensusParameters::<T>::iter_prefix(netuid)
            .min_by_key(|(k, _)| *k)
            .ok_or("No consensus parameters found")?;

        // Initialize Yuma epoch with the oldest parameters
        let mut yuma_epoch = YumaEpoch::<T>::new(netuid, oldest_params);

        // Execute Yuma consensus
        let emission_to_drain =
            Self::get_emission_to_drain(netuid).map_err(|_| "Failed to get emission to drain")?;
        yuma_epoch.run(weights).map_err(|_| "Failed to run Yuma consensus")?;

        // Apply consensus results
        yuma_epoch
            .params
            .apply(netuid)
            .map_err(|_| "Failed to apply consensus results")?;

        // Delete the processed ConsensusParameters from storage
        ConsensusParameters::<T>::remove(netuid, oldest_epoch);

        Ok(())
    }

    pub fn do_handle_authority_node_keep_alive(public_key: (Vec<u8>, Vec<u8>)) {
        if !Self::is_node_authorized(&public_key) {
            // TODO what to do here?
            return;
        }

        let mut authority_nodes = AuthorityNodes::<T>::get();

        let index = authority_nodes
            .iter()
            .position(|info| info.public_key == public_key)
            .unwrap_or(authority_nodes.len());
        authority_nodes.insert(
            index,
            AuthorityNodeInfo {
                public_key,
                last_keep_alive: pallet_subspace::Pallet::<T>::get_current_block_number(),
            },
        );

        AuthorityNodes::<T>::set(authority_nodes);
    }

    fn is_node_authorized(public_key: &(Vec<u8>, Vec<u8>)) -> bool {
        AuthorizedPublicKeys::<T>::get().iter().any(|node| node == public_key)
    }
}
