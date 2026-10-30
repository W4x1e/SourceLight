"""Ground truth for the spikes, labelled by eye from contact sheets of all 194
photos BEFORE any model output was looked at. Photo ids are library ids.

QUERIES: (query, relevant ids) — a photo is relevant when the thing asked for
is clearly in it. Phrased the way someone types into a search box.
TRANSLATED: the same needs in French / German / Spanish / Italian, scored
against the same sets — CLIP B/32 is English-only; SigLIP 2's text tower is
multilingual.
SCENES: every photo's scene, for "Similar" (is the nearest neighbour from the
same scene?) and near-duplicate separability.
"""

def r(a, b):
    return set(range(a, b + 1))


QUERIES = [
    ("coffee cup", {1}),
    ("brick viaduct with arches", r(2, 10)),
    ("woman with long blonde hair seen from behind", r(8, 11) | r(27, 35) | {92, 149, 150}),
    ("someone taking a photo with a camera", {11, 17, 18, 148}),
    ("white flowers close up", {19} | r(108, 111)),
    ("bee on a flower", r(108, 111) | {182, 183}),
    ("sunflower", {182, 183}),
    ("red railing on garden steps", r(36, 43)),
    ("melting ice cream", r(45, 49)),
    ("weeping willow", {44} | r(50, 59)),
    ("duck on a pond", {50, 51}),
    ("beach seen from a clifftop", r(65, 85)),
    ("man in a grey t-shirt looking at the view", {65, 66} | r(80, 83)),
    ("seagull", r(104, 107)),
    ("castle on a hill", r(112, 117) | r(120, 125)),
    ("stained glass window", {143, 144}),
    ("great hall with a wooden roof", r(128, 130)),
    ("harbour with small boats", r(167, 174)),
    ("yellow boat", r(167, 172)),
    ("clock tower", r(160, 162)),
    ("people jumping off the harbour wall", {165, 166}),
    ("picking tomatoes", r(184, 187)),
    ("woman in a red jacket", r(177, 181) | {188, 189}),
    ("hotel with red flowers on the balconies", r(190, 194)),
    ("bicycle", r(190, 194)),
    ("rocky cove with sea stacks", r(147, 155)),
    ("dusk by the water", r(86, 91)),
    ("sign pointing to the beach", r(101, 103)),
    ("man with a backpack", {92, 93} | r(95, 98) | {100}),
    ("narrow lane between houses", {63, 64}),
    ("bird on a tv aerial", {176}),
    ("woman wearing headphones", {175}),
    ("church door", {119, 145, 146}),
    ("aerial view of a town and causeway", r(139, 142)),
    ("concrete bunker", {126, 127}),
    ("path beside a yellow hedge", r(20, 22)),
    ("old boat moored at dusk", r(89, 91)),
    ("people on a beach on a cloudy day", r(92, 98) | r(112, 115)),
]

TRANSLATED = [
    ("tournesol", {182, 183}),                               # fr sunflower
    ("mouette", r(104, 107)),                                # fr seagull
    ("bateaux dans le port", r(167, 174)),                   # fr boats in the harbour
    ("Burg auf einem Hügel", r(112, 117) | r(120, 125)),     # de castle on a hill
    ("Biene auf einer Blume", r(108, 111) | {182, 183}),     # de bee on a flower
    ("Trauerweide", {44} | r(50, 59)),                       # de weeping willow
    ("helado derritiéndose", r(45, 49)),                     # es melting ice cream
    ("vidriera", {143, 144}),                                # es stained glass
    ("mujer con chaqueta roja", r(177, 181) | {188, 189}),   # es woman in a red jacket
    ("bicicletta", r(190, 194)),                             # it bicycle
    ("tramonto sull'acqua", r(86, 91)),                      # it dusk/sunset over water
    ("scala con ringhiera rossa", r(36, 43)),                # it stairs with red railing
]

_SCENES = [
    ("cafe", {1}), ("viaduct", r(2, 7)), ("blonde_pond", r(8, 10)), ("framed_garden", r(11, 16)),
    ("photographer", {17, 18}), ("hydrangea", {19}), ("hedge_path", r(20, 22)), ("pergola", r(23, 26)),
    ("balustrade", r(27, 35)), ("red_stairs", r(36, 43)), ("stone_bridge", {44}), ("ice_cream", r(45, 49)),
    ("willow_duck", {50, 51}), ("willow_path", {52, 53}), ("willow", r(54, 59)), ("sea_cliff", {60, 61}),
    ("cove", {62}), ("lane", {63, 64}), ("clifftop_bay", r(65, 85)), ("dusk_people", r(86, 88)),
    ("boat", r(89, 91)), ("beach_group", r(92, 98)), ("beach_path", {99, 100}), ("beach_sign", r(101, 103)),
    ("gull", r(104, 107)), ("bee", r(108, 111)), ("mount", r(112, 118)), ("chapel", {119}),
    ("castle", r(120, 125)), ("bunker", {126, 127}), ("hall", r(128, 130)), ("window", {131}),
    ("turret", r(132, 134)), ("shore_view", {135, 136}), ("parapet", {137}), ("church_ext", {138}),
    ("town_view", r(139, 142)), ("stained", {143, 144}), ("door_portrait", {145, 146}),
    ("kynance_people", r(147, 150)), ("kynance", r(151, 155)), ("street", {156}),
    ("harbour_pier", r(157, 159) | {165, 166}), ("clocktower", r(160, 162)), ("railing_people", {163, 164}),
    ("boats", r(167, 172) | {174}), ("old_man", {173}), ("headphones", {175}), ("aerial_bird", {176}),
    ("allotment", r(177, 181)), ("sunflower", {182, 183}), ("tomatoes", r(184, 187)),
    ("red_jacket_park", {188, 189}), ("hotel", r(190, 194)),
]
SCENE = {pid: name for name, ids in _SCENES for pid in ids}
assert sorted(SCENE) == list(range(1, 195)), "every photo needs exactly one scene"

# Near-duplicates: the same shot taken twice or a burst — what "Duplicates"
# should group. Everything else in the same scene is a *similar*, not a copy.
NEAR_DUPES = [
    (2, 3), (2, 5), (3, 5), (6, 7), (9, 10), (12, 13), (17, 18), (21, 22), (25, 26), (29, 30),
    (37, 38), (41, 42), (46, 47), (46, 48), (47, 48), (50, 51), (54, 55), (56, 57), (58, 59),
    (68, 69), (73, 74), (81, 82), (89, 90), (104, 105), (116, 117), (121, 122), (126, 127),
    (129, 130), (132, 133), (133, 134), (132, 134), (141, 142), (145, 146), (151, 152),
    (151, 153), (152, 153), (157, 158), (169, 170), (182, 183), (188, 189), (192, 193),
]
