"""A tiny CLI for manual warehouse operations, for ops on-call use."""

import sys

from reports import stock_report
from warehouse import Warehouse


def main(argv):
    if len(argv) < 2:
        print("usage: cli.py <initial_stock> <max_capacity>", file=sys.stderr)
        return 2
    warehouse = Warehouse(int(argv[0]), int(argv[1]))
    print(stock_report(warehouse))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
