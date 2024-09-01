import json
import argparse
import os
from typing import Any
from substrateinterface import SubstrateInterface

QUERY_URL: str = "wss://bittensor-finney.api.onfinality.io/public"
STANDARD_MODULE: str = "SubtensorModule"
SUBNET = 8
TEMPO = 100
START_BLOCK = 3_700_000
ITER_EPOCHS = 10



def query_map_values(
    client: SubstrateInterface,
    module: str,
    storage_function: str,
    params: list[Any] = [],
    block_hash: str | None = None
) -> dict[str, Any]:
    result = client.query_map(
        module=module, storage_function=storage_function, params=params, block_hash=block_hash # type: ignore
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


def get_weights(client: SubstrateInterface, block_hash: str) -> dict[str, dict[str, list[tuple[int, int]]]]:
    weights: dict[str, dict[str, list[tuple[int, int]]]] = {}

    subnet_weights = query_map_values(
        client, STANDARD_MODULE, "Weights", [SUBNET], block_hash
    )
    weights[str(SUBNET)] = {
        str(uid): [(int(target), int(weight)) for target, weight in w]
        for uid, w in subnet_weights.items()
    }

    return weights


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
    start_block_hash = client.get_block_hash(START_BLOCK)
    data['stake'] = get_stake(client, start_block_hash)

    data['weights'] = {}
    for i in range(ITER_EPOCHS):
        block_number = START_BLOCK + (i * TEMPO)
        block_hash = client.get_block_hash(block_number)
        data['weights'][str(block_number)] = get_weights(client, block_hash)
        print(f"Collected weights for block {block_number}")

    print(f"Writing snapshot to {output_path}")
    os.makedirs(args.directory, exist_ok=True)
    with open(output_path, "w") as f:
        json.dump(data, f, indent=4)

    print("Snapshot generation complete")


if __name__ == "__main__":
    main()
