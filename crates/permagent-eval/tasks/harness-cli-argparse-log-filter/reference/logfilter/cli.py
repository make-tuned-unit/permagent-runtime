import argparse
import sys

from logfilter.datespec import parse_date
from logfilter.filtering import VALID_LEVELS, filter_lines


def build_parser():
    p = argparse.ArgumentParser(prog="logfilter", description="Filter application log lines.")
    p.add_argument("logfile", help="path to the log file")
    p.add_argument("--level", help="only show lines at this level (debug/info/warn/error)")
    p.add_argument("--since", help="only show lines on/after this date (YYYY-MM-DD)")
    p.add_argument("--last", type=int, help="only show the last N matching lines")
    return p


def run(argv):
    parser = build_parser()
    args = parser.parse_args(argv)

    if args.since is not None and args.last is not None:
        print("error: --since and --last cannot be used together", file=sys.stderr)
        return 2

    if args.level is not None and args.level.lower() not in VALID_LEVELS:
        print(f"error: unknown level {args.level!r}", file=sys.stderr)
        return 2

    since = None
    if args.since is not None:
        try:
            since = parse_date(args.since)
        except ValueError:
            print(f"error: invalid --since date {args.since!r}, expected YYYY-MM-DD", file=sys.stderr)
            return 2

    if args.last is not None and args.last <= 0:
        print("error: --last must be a positive integer", file=sys.stderr)
        return 2

    try:
        with open(args.logfile) as f:
            lines = f.readlines()
    except OSError as exc:
        print(f"error: cannot read {args.logfile!r}: {exc}", file=sys.stderr)
        return 1

    matched = filter_lines(lines, level=args.level, since=since)

    if args.last is not None:
        matched = matched[-args.last:]

    for line in matched:
        print(line)
    return 0


def main(argv=None):
    return run(sys.argv[1:] if argv is None else argv)
