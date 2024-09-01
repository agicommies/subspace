from substrateinterface.storage import StorageKey
import json
import argparse
import os
from typing import Any
from substrateinterface import SubstrateInterface
from substrateinterface.exceptions import SubstrateRequestException

QUERY_URL: str = "wss://bittensor-finney.api.onfinality.io/public"
STANDARD_MODULE: str = "SubtensorModule"
SUBNET = 1
TEMPO = 100
START_BLOCK = 3_700_000
ITER_EPOCHS = 50


def query_map_values(
    client: SubstrateInterface,
    module: str,
    storage_function: str,
    params: list[Any] = [],
    block_hash: str | None = None
) -> dict[str, Any]:
    result = client.query_map(
        module=module, storage_function=storage_function, params=params, block_hash=block_hash  # type: ignore
    )
    return {str(k.value): v.value for k, v in result}

def get_stake(client: SubstrateInterface, block_hash: str) -> dict[str, int]:
    stake: dict[str, int] = {}

    all_uids = query_map_values(
        client,
        module=STANDARD_MODULE,
        storage_function="Uids",
        params=[SUBNET],
        block_hash=block_hash
    )

    for hotkey, uid in all_uids.items():
        try:
            stake_result = query_map_values(
                client,
                module=STANDARD_MODULE,
                storage_function="Stake",
                params=[hotkey],
                block_hash=block_hash
            )
            total_stake: int = sum(int(v) for v in stake_result.values())
            stake[str(uid)] = total_stake
        except Exception as e:
            print(f"Error querying stake for UID {str(uid)}: {str(e)}")
            stake[str(uid)] = 0

    return stake

def get_last_update(client: SubstrateInterface, block_hash: str) -> dict[str, str]:
    last_update = query_map_values(
        client, STANDARD_MODULE, "LastUpdate", [], block_hash
    )[str(SUBNET)]

    # uid to last update value
    sane_last_update: dict[str, str] = {}

    for uid, value in enumerate(last_update):
        sane_last_update[str(uid)] = value

    return sane_last_update


def get_registration_blocks(
    client: SubstrateInterface, block_hash: str
) -> dict[str, str]:

    registration_blocks = query_map_values(
        client, STANDARD_MODULE, "BlockAtRegistration", [SUBNET], block_hash
    )

    # uid to registration block value
    sane_registration_blocks: dict[str, str] = {}

    for uid, reg_block in registration_blocks.items():
        sane_registration_blocks[str(uid)] = reg_block

    return sane_registration_blocks


def get_epoch_data(
    client: SubstrateInterface, block_hash: str
) -> tuple[dict[str, dict[str, list[tuple[int, int]]]], dict[str, str], dict[str, str]]:
    weights: dict[str, dict[str, list[tuple[int, int]]]] = {}

    subnet_weights = query_map_values(
        client, STANDARD_MODULE, "Weights", [SUBNET], block_hash
    )
    weights[str(SUBNET)] = {
        str(uid): [(int(target), int(weight)) for target, weight in w]
        for uid, w in subnet_weights.items()
    }

    last_update = get_last_update(client, block_hash)
    registration_blocks = get_registration_blocks(client, block_hash)

    return weights, last_update, registration_blocks


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Generate a snapshot of weights and stake."
    )
    parser.add_argument(
        "-o", "--output", default="weights_stake.json", help="Output file name"
    )
    parser.add_argument("-d", "--directory", default=".",
                        help="Output directory")
    args: argparse.Namespace = parser.parse_args()

    output_path: str = os.path.join(args.directory, args.output)

    print("Starting snapshot generation...")
    client: SubstrateInterface = SubstrateInterface(QUERY_URL)
    print(f"Connected to {QUERY_URL}")

    data: dict[str, Any] = {}

    # Get initial stake from START_BLOCK
    print("hash")
    start_block_hash = client.get_block_hash(START_BLOCK)
    data["stake"] = get_stake(client, start_block_hash)
    print("stake")

    data["weights"] = {}
    data["last_update"] = {}
    data["registration_blocks"] = {}
    for i in range(ITER_EPOCHS):
        block_number = START_BLOCK + (i * TEMPO)
        block_hash = client.get_block_hash(block_number)
        weights, last_update, registration_blocks = get_epoch_data(
            client, block_hash)
        data["weights"][str(block_number)] = weights
        data["last_update"][str(block_number)] = last_update
        data["registration_blocks"][str(block_number)] = registration_blocks
        print(
            f"Collected weights, last update, and registration blocks for block {block_number}"
        )

    print(f"Writing snapshot to {output_path}")
    os.makedirs(args.directory, exist_ok=True)
    with open(output_path, "w") as f:
        json.dump(data, f, indent=4)

    print("Snapshot generation complete")


if __name__ == "__main__":
    main()
