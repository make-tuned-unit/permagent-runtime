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

    since = None
    if args.since:
        try:
            since = parse_date(args.since)
        except ValueError:
            since = None  # swallowed instead of reported

    try:
        with open(args.logfile) as f:
            lines = f.readlines()
    except OSError:
        lines = []  # swallowed instead of reported

    level = args.level  # not validated, not case-normalized

    matched = filter_lines(lines, level=level, since=since)

    if args.last is not None:
        matched = matched[-args.last:]  # zero/negative silently mis-slices

    for line in matched:
        print(line)
    return 0


def main(argv=None):
    return run(sys.argv[1:] if argv is None else argv)
