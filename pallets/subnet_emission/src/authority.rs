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

        // TODO check weight hashes
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
