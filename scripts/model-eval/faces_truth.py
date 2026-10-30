"""Who is who among the 43 faces the shipped pipeline detected in the library,
labelled by eye from context crops (indices are faces.jsonl line numbers).
Clothes were used only to confirm a face match, never to make one."""
IDENTITY = {
    "W1": [2, 3, 5, 9, 11, 12, 15, 27, 29, 31, 32],  # young woman, brown hair; hat + sunglasses in 95-98
    "W2": [4, 6],            # woman in pink, sunglasses
    "M1": [8, 10, 13, 14],   # man in cap, glasses
    "B":  [7, 22, 23],       # blonde woman
    "M2": [28, 30],          # older man, allotment (30 is mostly from behind)
    "W3": [24, 25],          # woman on the cliff
    "M5": [33, 34, 37, 40, 42],  # man in white jacket, bow tie (all tiny)
    "M6": [36, 38, 41],      # bearded man, dark shirt (all tiny)
}
SINGLES = [0, 1, 16, 17, 18, 19, 20, 21, 26, 35, 39]
LABEL = {i: k for k, v in IDENTITY.items() for i in v}
LABEL.update({i: f"s{i}" for i in SINGLES})
assert sorted(LABEL) == list(range(43))
