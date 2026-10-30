"""Ground truth for the OCR spike, by library id in the catalogue it
was labelled from (see the README: relabel for any other library), written from the photos themselves
(contact sheets, then each candidate at full preview size) BEFORE any OCR ran.

words: legible at the 1600 px preview and something a person could type to find
       the photo. Recall is measured on these.
maybe: small, partial, cursive, curved or cut-off text. Finding it is a bonus,
       missing it is not a miss, and reading it is never a false positive.
Every photo not listed shows no text at all: anything read there is a false
positive.
"""

BEACH = {"words": ["beach"], "maybe": []}
PAY = {"words": ["pay", "display"], "maybe": ["will", "be", "incurred"]}
PORTREATH = {"words": ["beach", "information"], "maybe": ["portreath", "west", "coast", "nz"]}
HARBOUR = {"words": ["harbour", "inn"], "maybe": ["porthleven"]}
POSTE = ["hotel", "de", "la", "poste", "the", "north", "face", "walk", "your", "bike", "biciclette", "mano", "14"]
KANTOR = {
    "words": ["gioielleria", "marco", "tadeusz", "emballage", "cricotage", "kantor",
              "collateral", "event", "biennale", "arte", "2026", "139"],
    "maybe": ["1915", "1990", "jar", "138", "s", "of", "ask", "me"],
}
DRONES = {
    "words": ["drone", "zone", "respect", "rules"],
    "maybe": ["no", "please", "the", "here", "you", "see", "of", "21", "drones", "found", "in", "short",
              "time", "our", "surroundings", "they", "are", "very", "noisy", "and", "disturbing", "people",
              "animals", "but", "also", "dangerous", "when", "crash", "pollute", "nature", "with",
              "plastic", "toxic", "contents", "lithium", "therefore", "leave", "your", "on", "ground",
              "as", "it", "is", "forbidden", "to", "fly", "national", "park", "questi", "sono", "dei",
              "droni", "che", "abbiamo", "trovato", "poco", "tempo", "nei", "nostri", "dintorni", "non",
              "solo", "rumorosi", "pericolosi", "ma", "come", "potete", "vedere", "inquinano", "le",
              "montagne", "con", "plastica", "metalli", "tossici", "lasciate", "vostri", "terra",
              "perche", "vietato", "volare", "nel", "parco", "naturale", "per", "favore", "rispettate",
              "regole", "hier", "seht", "ihr", "von", "drohnen", "die", "wir", "kurzester", "zeit",
              "unserer", "umgebung", "gefunden", "haben", "sie", "sind", "sehr", "larmreich", "storend",
              "aber", "auch", "gefahrlich", "wenn", "absturzen", "auberdem", "verpesten", "natur", "mit",
              "und", "toxischen", "inhalten", "deshalb", "lasst", "eure", "am", "boden", "da", "es",
              "verboten", "ist", "im", "naturpark", "zu", "fliegen", "bitte", "respektiert", "regeln",
              "air", "3", "6", "i", "a", "e", "si", "ne", "di", "da", "in", "ist", "offner", "nc", "no"],
}

TRUTH = {
    63: PORTREATH, 198: PORTREATH,
    87: PAY, 222: PAY,
    101: BEACH, 102: BEACH, 103: BEACH, 236: BEACH, 237: BEACH, 238: BEACH,
    156: HARBOUR, 291: HARBOUR,
    190: {"words": ["welcome"], "maybe": POSTE},
    191: {"words": ["welcome"], "maybe": POSTE},
    192: {"words": [], "maybe": POSTE}, 193: {"words": [], "maybe": POSTE},
    194: {"words": [], "maybe": POSTE}, 1123: {"words": [], "maybe": POSTE},
    1124: {"words": [], "maybe": POSTE},
    1128: KANTOR, 1129: KANTOR,
    725: DRONES, 727: DRONES,
    726: {"words": ["no", "picknick", "food"],
          "maybe": ["langalm", "2283", "toblach", "dobbiaco", "offner"] + DRONES["words"] + DRONES["maybe"]},
    658: {"words": ["bodenalm"], "maybe": []},
    996: {"words": ["bibliothek", "biblioteca"],
          "maybe": ["gebr", "baur", "strasse", "via", "f", "lli", "hotel", "restaurant", "villa"]},
    47: {"words": [], "maybe": []},  # a cup's brand, printed upside down
    1109: {"words": ["welcome", "adidas", "guess"], "maybe": ["cortina"]},
    1184: {"words": ["garden", "bar"], "maybe": []},
    1176: {"words": ["garden", "bar"], "maybe": []},
    1204: {"words": ["actv"], "maybe": ["pt", "dema"]},
    1205: {"words": ["actv"], "maybe": ["pt", "dema"]},
    1034: {"words": ["tofana"], "maybe": []},
    1031: {"words": ["tofana"], "maybe": []},
    1095: {"words": ["shell", "ups"], "maybe": ["ferrari", "f1", "puma"]},
    730: {"words": [], "maybe": []},  # a chalkboard menu, unreadable at this size
    1022: {"words": [], "maybe": ["4.6", "46", "600", "pistenbully"]},
    1012: {"words": [], "maybe": ["caneva", "canevi"]},
    604: {"words": [], "maybe": ["terrex"]},
    338: {"words": ["terrex", "42"], "maybe": ["mcdonald", "mcdonalds"]},
    401: {"words": ["premiata"], "maybe": []},
    298: {"words": [], "maybe": ["shipyard", "hipyard", "port", "porthleven"]},
    846: {"words": ["burnt", "studios"], "maybe": ["athl", "dept"]},
    307: {"words": ["mizzi", "pz", "18"], "maybe": ["la", "chat", "lachat", "harbour", "inn", "wizard", "katy"]},
    325: {"words": ["victoria", "mountain", "shop"], "maybe": ["cortina", "reflective", "system", "welcome"]},
    # Found by OCR and confirmed by eye AFTER the run: real text missed when
    # labelling (a jacket's logo, an outboard motor, a boat's number far off).
    # Listed as maybe, so reading them is neither a hit nor a false positive.
    530: {"words": [], "maybe": ["terrex", "aterre"]},
    1243: {"words": [], "maybe": ["selva"]},
    171: {"words": [], "maybe": ["pz", "18", "pz18"]},
    326: {"words": ["victoria", "ztl", "attiva", "closed"],
          "maybe": ["welcome", "hotel", "concordia", "ristorante", "e", "bar", "dal", "principe", "cortina", "13"]},
}
