def rank_players(scores):
    """Rank players by score, highest first, using standard competition
    ranking (ties share a rank; the next distinct score skips ahead by
    however many players tied).

    `scores` is a list of (name, score) pairs. Returns a list of
    (name, rank) pairs in the SAME order as the input.
    """
    ordered = sorted(scores, key=lambda p: p[1], reverse=True)
    return [(name, i + 1) for i, (name, score) in enumerate(ordered)]
