use std::{fmt, str, str::FromStr};

use bitvec::{field::BitField, order::Msb0, slice::BitSlice, vec::BitVec, view::BitView};
use nimiq_hash::{
    pbkdf2::{compute_pbkdf2_sha512, Pbkdf2Error},
    Blake2bHasher, HashOutput, Hasher, Sha256Hasher,
};
use nimiq_macros::{add_hex_io_fns_typed_arr, create_typed_array};
use nimiq_utils::{
    crc::Crc8Computer,
    key_rng::SecureGenerate,
    otp::{otp, Algorithm},
};
use rand_core::{CryptoRng, RngCore};
use unicode_normalization::UnicodeNormalization;

#[cfg(feature = "key-derivation")]
pub mod key_derivation;

// Our entropy implementation is fixed to 32 bytes.
create_typed_array!(Entropy, u8, 32);
add_hex_io_fns_typed_arr!(Entropy, 32);

impl Entropy {
    const CHECKSUM_SIZE: usize = 8;

    fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Computes the CRC8 checksum of this entropy.
    fn crc8_checksum(&self) -> u8 {
        Crc8Computer::default().update(self.as_bytes()).result()
    }

    /// Computes the sha256 checksum of this entropy.
    fn sha256_checksum(&self) -> u8 {
        let hash = Sha256Hasher::default().digest(self.as_bytes());
        hash.as_bytes()[0]
    }

    /// Checks for collisions between legacy and BIP39 style checksums.
    #[inline]
    pub fn is_colliding_checksum(&self) -> bool {
        self.crc8_checksum() == self.sha256_checksum()
    }

    /// Creates a Mnemonic out of this entropy (BIP39 compatible).
    pub fn to_mnemonic(&self, wordlist: Wordlist) -> Mnemonic {
        let mut bits = BitVec::with_capacity(Self::SIZE + Self::CHECKSUM_SIZE);
        bits.extend_from_bitslice(self.as_bytes().view_bits::<Msb0>());
        bits.extend_from_bitslice(self.sha256_checksum().view_bits::<Msb0>());
        Mnemonic::from_bits(&bits, wordlist)
    }

    /// Creates a Mnemonic out of this entropy using the legacy style checksum.
    #[deprecated(since = "0.0.1", note = "please use `to_mnemonic` instead")]
    pub fn to_legacy_mnemonic(&self, wordlist: Wordlist) -> Mnemonic {
        let mut bits = BitVec::with_capacity(Self::SIZE + Self::CHECKSUM_SIZE);
        bits.extend_from_bitslice(self.as_bytes().view_bits::<Msb0>());
        bits.extend_from_bitslice(self.crc8_checksum().view_bits::<Msb0>());
        Mnemonic::from_bits(&bits, wordlist)
    }
}

impl SecureGenerate for Entropy {
    fn generate<R: RngCore + CryptoRng>(rng: &mut R) -> Self {
        let mut bytes = [0u8; Entropy::SIZE];
        rng.fill_bytes(&mut bytes[..]);
        bytes.into()
    }
}

impl Entropy {
    const PURPOSE_ID: u32 = 0x42000002;
    const ENCRYPTION_SALT_SIZE: usize = 16;
    const ENCRYPTION_KDF_ROUNDS: u32 = 256;
    const ENCRYPTION_CHECKSUM_SIZE_V3: usize = 2;

    pub fn export_encrypted(&self, key: &[u8]) -> Result<Vec<u8>, String> {
        let mut salt = [0u8; Entropy::ENCRYPTION_SALT_SIZE];
        rand_core::OsRng.fill_bytes(&mut salt);

        let mut data = Vec::with_capacity(/*purposeId*/ 4 + Entropy::SIZE);
        data.extend_from_slice(&Entropy::PURPOSE_ID.to_be_bytes());
        data.extend_from_slice(self.as_bytes());

        let checksum: [u8; Entropy::ENCRYPTION_CHECKSUM_SIZE_V3] = Blake2bHasher::default()
            .digest(&data)
            .as_bytes()[0..Entropy::ENCRYPTION_CHECKSUM_SIZE_V3]
            .try_into()
            .unwrap();
        let mut plaintext = Vec::with_capacity(checksum.len() + data.len());
        plaintext.extend_from_slice(&checksum);
        plaintext.extend_from_slice(&data);
        let ciphertext = otp(
            &plaintext,
            key,
            Entropy::ENCRYPTION_KDF_ROUNDS,
            &salt,
            Algorithm::Argon2d,
        )
        .map_err(|err| format!("{:?}", err))?;

        let mut buf = Vec::with_capacity(
            /*version*/ 1 + /*kdf rounds*/ 1 + salt.len() + ciphertext.len(),
        );
        buf.extend_from_slice(&3u8.to_be_bytes()); // version
        buf.extend_from_slice(
            &((Entropy::ENCRYPTION_KDF_ROUNDS as f32).log2() as u8).to_be_bytes(),
        );
        buf.extend_from_slice(&salt);
        buf.extend_from_slice(&ciphertext);

        Ok(buf)
    }

    pub fn from_encrypted(buf: &[u8], key: &[u8]) -> Result<Entropy, String> {
        let version = buf[0];
        let rounds_log = buf[1];
        if rounds_log > 32 {
            return Err(format!("Rounds out-of-bounds: 2^{}", rounds_log));
        }
        let rounds = 2u32.pow(rounds_log as u32);

        match version {
            1 => unimplemented!(),
            2 => unimplemented!(),
            3 => Entropy::decrypt_v3(&buf[2..], key, rounds),
            _ => Err(format!("Unknown version: {}", version)),
        }
    }

    fn decrypt_v3(buf: &[u8], key: &[u8], rounds: u32) -> Result<Entropy, String> {
        if buf.len() < Entropy::ENCRYPTION_SALT_SIZE {
            return Err("Buffer too short".to_string());
        }
        let salt = &buf[..Entropy::ENCRYPTION_SALT_SIZE];
        let ciphertext = &buf[Entropy::ENCRYPTION_SALT_SIZE
            ..Entropy::ENCRYPTION_SALT_SIZE + Entropy::ENCRYPTION_CHECKSUM_SIZE_V3 + /*purposeId*/ 4 + Entropy::SIZE];

        let plaintext = otp(ciphertext, key, rounds, salt, Algorithm::Argon2d)
            .map_err(|err| format!("{:?}", err))?;

        let check = &plaintext[..Entropy::ENCRYPTION_CHECKSUM_SIZE_V3];
        let payload = &plaintext[Entropy::ENCRYPTION_CHECKSUM_SIZE_V3..];
        let checksum: [u8; Entropy::ENCRYPTION_CHECKSUM_SIZE_V3] = Blake2bHasher::default()
            .digest(payload)
            .as_bytes()[..Entropy::ENCRYPTION_CHECKSUM_SIZE_V3]
            .try_into()
            .unwrap();
        if checksum != check {
            return Err("Invalid key".to_string());
        }

        let purpose_id = u32::from_be_bytes(payload[..4].try_into().unwrap());
        if purpose_id != Entropy::PURPOSE_ID {
            return Err(format!("Invalid secret type (Purpose ID) {}", purpose_id));
        }

        let secret = &payload[4..];
        Ok(Entropy::from(secret))
    }
}

const WORDLIST_SIZE: usize = 2048;
pub type Wordlist<'a> = [&'a str; WORDLIST_SIZE];
pub const WORDLIST_EN: Wordlist<'static> = [
    "abandon", "ability", "able", "about", "above", "absent", "absorb", "abstract", "absurd",
    "abuse", "access", "accident", "account", "accuse", "achieve", "acid", "acoustic", "acquire",
    "across", "act", "action", "actor", "actress", "actual", "adapt", "add", "addict", "address",
    "adjust", "admit", "adult", "advance", "advice", "aerobic", "affair", "afford", "afraid",
    "again", "age", "agent", "agree", "ahead", "aim", "air", "airport", "aisle", "alarm", "album",
    "alcohol", "alert", "alien", "all", "alley", "allow", "almost", "alone", "alpha", "already",
    "also", "alter", "always", "amateur", "amazing", "among", "amount", "amused", "analyst",
    "anchor", "ancient", "anger", "angle", "angry", "animal", "ankle", "announce", "annual",
    "another", "answer", "antenna", "antique", "anxiety", "any", "apart", "apology", "appear",
    "apple", "approve", "april", "arch", "arctic", "area", "arena", "argue", "arm", "armed",
    "armor", "army", "around", "arrange", "arrest", "arrive", "arrow", "art", "artefact", "artist",
    "artwork", "ask", "aspect", "assault", "asset", "assist", "assume", "asthma", "athlete",
    "atom", "attack", "attend", "attitude", "attract", "auction", "audit", "august", "aunt",
    "author", "auto", "autumn", "average", "avocado", "avoid", "awake", "aware", "away", "awesome",
    "awful", "awkward", "axis", "baby", "bachelor", "bacon", "badge", "bag", "balance", "balcony",
    "ball", "bamboo", "banana", "banner", "bar", "barely", "bargain", "barrel", "base", "basic",
    "basket", "battle", "beach", "bean", "beauty", "because", "become", "beef", "before", "begin",
    "behave", "behind", "believe", "below", "belt", "bench", "benefit", "best", "betray", "better",
    "between", "beyond", "bicycle", "bid", "bike", "bind", "biology", "bird", "birth", "bitter",
    "black", "blade", "blame", "blanket", "blast", "bleak", "bless", "blind", "blood", "blossom",
    "blouse", "blue", "blur", "blush", "board", "boat", "body", "boil", "bomb", "bone", "bonus",
    "book", "boost", "border", "boring", "borrow", "boss", "bottom", "bounce", "box", "boy",
    "bracket", "brain", "brand", "brass", "brave", "bread", "breeze", "brick", "bridge", "brief",
    "bright", "bring", "brisk", "broccoli", "broken", "bronze", "broom", "brother", "brown",
    "brush", "bubble", "buddy", "budget", "buffalo", "build", "bulb", "bulk", "bullet", "bundle",
    "bunker", "burden", "burger", "burst", "bus", "business", "busy", "butter", "buyer", "buzz",
    "cabbage", "cabin", "cable", "cactus", "cage", "cake", "call", "calm", "camera", "camp", "can",
    "canal", "cancel", "candy", "cannon", "canoe", "canvas", "canyon", "capable", "capital",
    "captain", "car", "carbon", "card", "cargo", "carpet", "carry", "cart", "case", "cash",
    "casino", "castle", "casual", "cat", "catalog", "catch", "category", "cattle", "caught",
    "cause", "caution", "cave", "ceiling", "celery", "cement", "census", "century", "cereal",
    "certain", "chair", "chalk", "champion", "change", "chaos", "chapter", "charge", "chase",
    "chat", "cheap", "check", "cheese", "chef", "cherry", "chest", "chicken", "chief", "child",
    "chimney", "choice", "choose", "chronic", "chuckle", "chunk", "churn", "cigar", "cinnamon",
    "circle", "citizen", "city", "civil", "claim", "clap", "clarify", "claw", "clay", "clean",
    "clerk", "clever", "click", "client", "cliff", "climb", "clinic", "clip", "clock", "clog",
    "close", "cloth", "cloud", "clown", "club", "clump", "cluster", "clutch", "coach", "coast",
    "coconut", "code", "coffee", "coil", "coin", "collect", "color", "column", "combine", "come",
    "comfort", "comic", "common", "company", "concert", "conduct", "confirm", "congress",
    "connect", "consider", "control", "convince", "cook", "cool", "copper", "copy", "coral",
    "core", "corn", "correct", "cost", "cotton", "couch", "country", "couple", "course", "cousin",
    "cover", "coyote", "crack", "cradle", "craft", "cram", "crane", "crash", "crater", "crawl",
    "crazy", "cream", "credit", "creek", "crew", "cricket", "crime", "crisp", "critic", "crop",
    "cross", "crouch", "crowd", "crucial", "cruel", "cruise", "crumble", "crunch", "crush", "cry",
    "crystal", "cube", "culture", "cup", "cupboard", "curious", "current", "curtain", "curve",
    "cushion", "custom", "cute", "cycle", "dad", "damage", "damp", "dance", "danger", "daring",
    "dash", "daughter", "dawn", "day", "deal", "debate", "debris", "decade", "december", "decide",
    "decline", "decorate", "decrease", "deer", "defense", "define", "defy", "degree", "delay",
    "deliver", "demand", "demise", "denial", "dentist", "deny", "depart", "depend", "deposit",
    "depth", "deputy", "derive", "describe", "desert", "design", "desk", "despair", "destroy",
    "detail", "detect", "develop", "device", "devote", "diagram", "dial", "diamond", "diary",
    "dice", "diesel", "diet", "differ", "digital", "dignity", "dilemma", "dinner", "dinosaur",
    "direct", "dirt", "disagree", "discover", "disease", "dish", "dismiss", "disorder", "display",
    "distance", "divert", "divide", "divorce", "dizzy", "doctor", "document", "dog", "doll",
    "dolphin", "domain", "donate", "donkey", "donor", "door", "dose", "double", "dove", "draft",
    "dragon", "drama", "drastic", "draw", "dream", "dress", "drift", "drill", "drink", "drip",
    "drive", "drop", "drum", "dry", "duck", "dumb", "dune", "during", "dust", "dutch", "duty",
    "dwarf", "dynamic", "eager", "eagle", "early", "earn", "earth", "easily", "east", "easy",
    "echo", "ecology", "economy", "edge", "edit", "educate", "effort", "egg", "eight", "either",
    "elbow", "elder", "electric", "elegant", "element", "elephant", "elevator", "elite", "else",
    "embark", "embody", "embrace", "emerge", "emotion", "employ", "empower", "empty", "enable",
    "enact", "end", "endless", "endorse", "enemy", "energy", "enforce", "engage", "engine",
    "enhance", "enjoy", "enlist", "enough", "enrich", "enroll", "ensure", "enter", "entire",
    "entry", "envelope", "episode", "equal", "equip", "era", "erase", "erode", "erosion", "error",
    "erupt", "escape", "essay", "essence", "estate", "eternal", "ethics", "evidence", "evil",
    "evoke", "evolve", "exact", "example", "excess", "exchange", "excite", "exclude", "excuse",
    "execute", "exercise", "exhaust", "exhibit", "exile", "exist", "exit", "exotic", "expand",
    "expect", "expire", "explain", "expose", "express", "extend", "extra", "eye", "eyebrow",
    "fabric", "face", "faculty", "fade", "faint", "faith", "fall", "false", "fame", "family",
    "famous", "fan", "fancy", "fantasy", "farm", "fashion", "fat", "fatal", "father", "fatigue",
    "fault", "favorite", "feature", "february", "federal", "fee", "feed", "feel", "female",
    "fence", "festival", "fetch", "fever", "few", "fiber", "fiction", "field", "figure", "file",
    "film", "filter", "final", "find", "fine", "finger", "finish", "fire", "firm", "first",
    "fiscal", "fish", "fit", "fitness", "fix", "flag", "flame", "flash", "flat", "flavor", "flee",
    "flight", "flip", "float", "flock", "floor", "flower", "fluid", "flush", "fly", "foam",
    "focus", "fog", "foil", "fold", "follow", "food", "foot", "force", "forest", "forget", "fork",
    "fortune", "forum", "forward", "fossil", "foster", "found", "fox", "fragile", "frame",
    "frequent", "fresh", "friend", "fringe", "frog", "front", "frost", "frown", "frozen", "fruit",
    "fuel", "fun", "funny", "furnace", "fury", "future", "gadget", "gain", "galaxy", "gallery",
    "game", "gap", "garage", "garbage", "garden", "garlic", "garment", "gas", "gasp", "gate",
    "gather", "gauge", "gaze", "general", "genius", "genre", "gentle", "genuine", "gesture",
    "ghost", "giant", "gift", "giggle", "ginger", "giraffe", "girl", "give", "glad", "glance",
    "glare", "glass", "glide", "glimpse", "globe", "gloom", "glory", "glove", "glow", "glue",
    "goat", "goddess", "gold", "good", "goose", "gorilla", "gospel", "gossip", "govern", "gown",
    "grab", "grace", "grain", "grant", "grape", "grass", "gravity", "great", "green", "grid",
    "grief", "grit", "grocery", "group", "grow", "grunt", "guard", "guess", "guide", "guilt",
    "guitar", "gun", "gym", "habit", "hair", "half", "hammer", "hamster", "hand", "happy",
    "harbor", "hard", "harsh", "harvest", "hat", "have", "hawk", "hazard", "head", "health",
    "heart", "heavy", "hedgehog", "height", "hello", "helmet", "help", "hen", "hero", "hidden",
    "high", "hill", "hint", "hip", "hire", "history", "hobby", "hockey", "hold", "hole", "holiday",
    "hollow", "home", "honey", "hood", "hope", "horn", "horror", "horse", "hospital", "host",
    "hotel", "hour", "hover", "hub", "huge", "human", "humble", "humor", "hundred", "hungry",
    "hunt", "hurdle", "hurry", "hurt", "husband", "hybrid", "ice", "icon", "idea", "identify",
    "idle", "ignore", "ill", "illegal", "illness", "image", "imitate", "immense", "immune",
    "impact", "impose", "improve", "impulse", "inch", "include", "income", "increase", "index",
    "indicate", "indoor", "industry", "infant", "inflict", "inform", "inhale", "inherit",
    "initial", "inject", "injury", "inmate", "inner", "innocent", "input", "inquiry", "insane",
    "insect", "inside", "inspire", "install", "intact", "interest", "into", "invest", "invite",
    "involve", "iron", "island", "isolate", "issue", "item", "ivory", "jacket", "jaguar", "jar",
    "jazz", "jealous", "jeans", "jelly", "jewel", "job", "join", "joke", "journey", "joy", "judge",
    "juice", "jump", "jungle", "junior", "junk", "just", "kangaroo", "keen", "keep", "ketchup",
    "key", "kick", "kid", "kidney", "kind", "kingdom", "kiss", "kit", "kitchen", "kite", "kitten",
    "kiwi", "knee", "knife", "knock", "know", "lab", "label", "labor", "ladder", "lady", "lake",
    "lamp", "language", "laptop", "large", "later", "latin", "laugh", "laundry", "lava", "law",
    "lawn", "lawsuit", "layer", "lazy", "leader", "leaf", "learn", "leave", "lecture", "left",
    "leg", "legal", "legend", "leisure", "lemon", "lend", "length", "lens", "leopard", "lesson",
    "letter", "level", "liar", "liberty", "library", "license", "life", "lift", "light", "like",
    "limb", "limit", "link", "lion", "liquid", "list", "little", "live", "lizard", "load", "loan",
    "lobster", "local", "lock", "logic", "lonely", "long", "loop", "lottery", "loud", "lounge",
    "love", "loyal", "lucky", "luggage", "lumber", "lunar", "lunch", "luxury", "lyrics", "machine",
    "mad", "magic", "magnet", "maid", "mail", "main", "major", "make", "mammal", "man", "manage",
    "mandate", "mango", "mansion", "manual", "maple", "marble", "march", "margin", "marine",
    "market", "marriage", "mask", "mass", "master", "match", "material", "math", "matrix",
    "matter", "maximum", "maze", "meadow", "mean", "measure", "meat", "mechanic", "medal", "media",
    "melody", "melt", "member", "memory", "mention", "menu", "mercy", "merge", "merit", "merry",
    "mesh", "message", "metal", "method", "middle", "midnight", "milk", "million", "mimic", "mind",
    "minimum", "minor", "minute", "miracle", "mirror", "misery", "miss", "mistake", "mix", "mixed",
    "mixture", "mobile", "model", "modify", "mom", "moment", "monitor", "monkey", "monster",
    "month", "moon", "moral", "more", "morning", "mosquito", "mother", "motion", "motor",
    "mountain", "mouse", "move", "movie", "much", "muffin", "mule", "multiply", "muscle", "museum",
    "mushroom", "music", "must", "mutual", "myself", "mystery", "myth", "naive", "name", "napkin",
    "narrow", "nasty", "nation", "nature", "near", "neck", "need", "negative", "neglect",
    "neither", "nephew", "nerve", "nest", "net", "network", "neutral", "never", "news", "next",
    "nice", "night", "noble", "noise", "nominee", "noodle", "normal", "north", "nose", "notable",
    "note", "nothing", "notice", "novel", "now", "nuclear", "number", "nurse", "nut", "oak",
    "obey", "object", "oblige", "obscure", "observe", "obtain", "obvious", "occur", "ocean",
    "october", "odor", "off", "offer", "office", "often", "oil", "okay", "old", "olive", "olympic",
    "omit", "once", "one", "onion", "online", "only", "open", "opera", "opinion", "oppose",
    "option", "orange", "orbit", "orchard", "order", "ordinary", "organ", "orient", "original",
    "orphan", "ostrich", "other", "outdoor", "outer", "output", "outside", "oval", "oven", "over",
    "own", "owner", "oxygen", "oyster", "ozone", "pact", "paddle", "page", "pair", "palace",
    "palm", "panda", "panel", "panic", "panther", "paper", "parade", "parent", "park", "parrot",
    "party", "pass", "patch", "path", "patient", "patrol", "pattern", "pause", "pave", "payment",
    "peace", "peanut", "pear", "peasant", "pelican", "pen", "penalty", "pencil", "people",
    "pepper", "perfect", "permit", "person", "pet", "phone", "photo", "phrase", "physical",
    "piano", "picnic", "picture", "piece", "pig", "pigeon", "pill", "pilot", "pink", "pioneer",
    "pipe", "pistol", "pitch", "pizza", "place", "planet", "plastic", "plate", "play", "please",
    "pledge", "pluck", "plug", "plunge", "poem", "poet", "point", "polar", "pole", "police",
    "pond", "pony", "pool", "popular", "portion", "position", "possible", "post", "potato",
    "pottery", "poverty", "powder", "power", "practice", "praise", "predict", "prefer", "prepare",
    "present", "pretty", "prevent", "price", "pride", "primary", "print", "priority", "prison",
    "private", "prize", "problem", "process", "produce", "profit", "program", "project", "promote",
    "proof", "property", "prosper", "protect", "proud", "provide", "public", "pudding", "pull",
    "pulp", "pulse", "pumpkin", "punch", "pupil", "puppy", "purchase", "purity", "purpose",
    "purse", "push", "put", "puzzle", "pyramid", "quality", "quantum", "quarter", "question",
    "quick", "quit", "quiz", "quote", "rabbit", "raccoon", "race", "rack", "radar", "radio",
    "rail", "rain", "raise", "rally", "ramp", "ranch", "random", "range", "rapid", "rare", "rate",
    "rather", "raven", "raw", "razor", "ready", "real", "reason", "rebel", "rebuild", "recall",
    "receive", "recipe", "record", "recycle", "reduce", "reflect", "reform", "refuse", "region",
    "regret", "regular", "reject", "relax", "release", "relief", "rely", "remain", "remember",
    "remind", "remove", "render", "renew", "rent", "reopen", "repair", "repeat", "replace",
    "report", "require", "rescue", "resemble", "resist", "resource", "response", "result",
    "retire", "retreat", "return", "reunion", "reveal", "review", "reward", "rhythm", "rib",
    "ribbon", "rice", "rich", "ride", "ridge", "rifle", "right", "rigid", "ring", "riot", "ripple",
    "risk", "ritual", "rival", "river", "road", "roast", "robot", "robust", "rocket", "romance",
    "roof", "rookie", "room", "rose", "rotate", "rough", "round", "route", "royal", "rubber",
    "rude", "rug", "rule", "run", "runway", "rural", "sad", "saddle", "sadness", "safe", "sail",
    "salad", "salmon", "salon", "salt", "salute", "same", "sample", "sand", "satisfy", "satoshi",
    "sauce", "sausage", "save", "say", "scale", "scan", "scare", "scatter", "scene", "scheme",
    "school", "science", "scissors", "scorpion", "scout", "scrap", "screen", "script", "scrub",
    "sea", "search", "season", "seat", "second", "secret", "section", "security", "seed", "seek",
    "segment", "select", "sell", "seminar", "senior", "sense", "sentence", "series", "service",
    "session", "settle", "setup", "seven", "shadow", "shaft", "shallow", "share", "shed", "shell",
    "sheriff", "shield", "shift", "shine", "ship", "shiver", "shock", "shoe", "shoot", "shop",
    "short", "shoulder", "shove", "shrimp", "shrug", "shuffle", "shy", "sibling", "sick", "side",
    "siege", "sight", "sign", "silent", "silk", "silly", "silver", "similar", "simple", "since",
    "sing", "siren", "sister", "situate", "six", "size", "skate", "sketch", "ski", "skill", "skin",
    "skirt", "skull", "slab", "slam", "sleep", "slender", "slice", "slide", "slight", "slim",
    "slogan", "slot", "slow", "slush", "small", "smart", "smile", "smoke", "smooth", "snack",
    "snake", "snap", "sniff", "snow", "soap", "soccer", "social", "sock", "soda", "soft", "solar",
    "soldier", "solid", "solution", "solve", "someone", "song", "soon", "sorry", "sort", "soul",
    "sound", "soup", "source", "south", "space", "spare", "spatial", "spawn", "speak", "special",
    "speed", "spell", "spend", "sphere", "spice", "spider", "spike", "spin", "spirit", "split",
    "spoil", "sponsor", "spoon", "sport", "spot", "spray", "spread", "spring", "spy", "square",
    "squeeze", "squirrel", "stable", "stadium", "staff", "stage", "stairs", "stamp", "stand",
    "start", "state", "stay", "steak", "steel", "stem", "step", "stereo", "stick", "still",
    "sting", "stock", "stomach", "stone", "stool", "story", "stove", "strategy", "street",
    "strike", "strong", "struggle", "student", "stuff", "stumble", "style", "subject", "submit",
    "subway", "success", "such", "sudden", "suffer", "sugar", "suggest", "suit", "summer", "sun",
    "sunny", "sunset", "super", "supply", "supreme", "sure", "surface", "surge", "surprise",
    "surround", "survey", "suspect", "sustain", "swallow", "swamp", "swap", "swarm", "swear",
    "sweet", "swift", "swim", "swing", "switch", "sword", "symbol", "symptom", "syrup", "system",
    "table", "tackle", "tag", "tail", "talent", "talk", "tank", "tape", "target", "task", "taste",
    "tattoo", "taxi", "teach", "team", "tell", "ten", "tenant", "tennis", "tent", "term", "test",
    "text", "thank", "that", "theme", "then", "theory", "there", "they", "thing", "this",
    "thought", "three", "thrive", "throw", "thumb", "thunder", "ticket", "tide", "tiger", "tilt",
    "timber", "time", "tiny", "tip", "tired", "tissue", "title", "toast", "tobacco", "today",
    "toddler", "toe", "together", "toilet", "token", "tomato", "tomorrow", "tone", "tongue",
    "tonight", "tool", "tooth", "top", "topic", "topple", "torch", "tornado", "tortoise", "toss",
    "total", "tourist", "toward", "tower", "town", "toy", "track", "trade", "traffic", "tragic",
    "train", "transfer", "trap", "trash", "travel", "tray", "treat", "tree", "trend", "trial",
    "tribe", "trick", "trigger", "trim", "trip", "trophy", "trouble", "truck", "true", "truly",
    "trumpet", "trust", "truth", "try", "tube", "tuition", "tumble", "tuna", "tunnel", "turkey",
    "turn", "turtle", "twelve", "twenty", "twice", "twin", "twist", "two", "type", "typical",
    "ugly", "umbrella", "unable", "unaware", "uncle", "uncover", "under", "undo", "unfair",
    "unfold", "unhappy", "uniform", "unique", "unit", "universe", "unknown", "unlock", "until",
    "unusual", "unveil", "update", "upgrade", "uphold", "upon", "upper", "upset", "urban", "urge",
    "usage", "use", "used", "useful", "useless", "usual", "utility", "vacant", "vacuum", "vague",
    "valid", "valley", "valve", "van", "vanish", "vapor", "various", "vast", "vault", "vehicle",
    "velvet", "vendor", "venture", "venue", "verb", "verify", "version", "very", "vessel",
    "veteran", "viable", "vibrant", "vicious", "victory", "video", "view", "village", "vintage",
    "violin", "virtual", "virus", "visa", "visit", "visual", "vital", "vivid", "vocal", "voice",
    "void", "volcano", "volume", "vote", "voyage", "wage", "wagon", "wait", "walk", "wall",
    "walnut", "want", "warfare", "warm", "warrior", "wash", "wasp", "waste", "water", "wave",
    "way", "wealth", "weapon", "wear", "weasel", "weather", "web", "wedding", "weekend", "weird",
    "welcome", "west", "wet", "whale", "what", "wheat", "wheel", "when", "where", "whip",
    "whisper", "wide", "width", "wife", "wild", "will", "win", "window", "wine", "wing", "wink",
    "winner", "winter", "wire", "wisdom", "wise", "wish", "witness", "wolf", "woman", "wonder",
    "wood", "wool", "word", "work", "world", "worry", "worth", "wrap", "wreck", "wrestle", "wrist",
    "write", "wrong", "yard", "year", "yellow", "you", "young", "youth", "zebra", "zero", "zone",
    "zoo",
];

#[derive(Clone, Ord, PartialOrd, Eq, PartialEq, Debug)]
pub struct Mnemonic {
    mnemonic: Vec<String>,
}

fn push_usize(bit_vec: &mut BitVec<u8, Msb0>, value: usize, nbits: usize) {
    for i in (0..nbits).rev() {
        bit_vec.push((value >> i) & 1 == 1);
    }
}

impl Mnemonic {
    const NUM_BITS_PER_WORD: usize = 11;

    /// Creates a Mnemonic out of a BitVec.
    fn from_bits(bits: &BitSlice<u8, Msb0>, wordlist: Wordlist) -> Self {
        let mnemonic = bits
            .chunks(Self::NUM_BITS_PER_WORD)
            .map(|chunk| {
                let mut index = 0u16;
                // Copy bits into index variable
                let index_bits = index.view_bits_mut::<Msb0>();
                index_bits[16 - Self::NUM_BITS_PER_WORD..].clone_from_bitslice(chunk);
                wordlist[index as usize].to_string()
            })
            .collect::<Vec<String>>();
        Mnemonic { mnemonic }
    }

    /// Tries to convert a Mnemonic into a BitVec. This may fail if the Mnemonic doesn't match the wordlist.
    fn to_bits(&self, wordlist: Wordlist) -> Option<BitVec<u8, Msb0>> {
        let mut bit_vec = BitVec::with_capacity(self.mnemonic.len() * Self::NUM_BITS_PER_WORD);
        for index in self
            .mnemonic
            .iter()
            .map(|word| wordlist.binary_search(&word.to_lowercase().as_ref()).ok())
        {
            match index {
                Some(i) => push_usize(&mut bit_vec, i, Self::NUM_BITS_PER_WORD),
                None => return None,
            }
        }
        Some(bit_vec)
    }

    /// Creates an entropy out of this mnemonic.
    fn bits_to_entropy_generic(bits: &BitSlice<u8, Msb0>, legacy: bool) -> Option<Entropy> {
        // Split up bits into entropy and checksum.
        let mut checksum_len = bits.len() % 8;
        if checksum_len == 0 {
            checksum_len = 8;
        }
        let divider_index = bits.len() - checksum_len;

        // Convert checksum and truncate bits.
        let entropy_bits = &bits[..divider_index];
        let mut checksum = 0u8;
        checksum
            .view_bits_mut()
            .copy_from_bitslice(&bits[divider_index..divider_index + checksum_len]);

        // Convert the bitslice to a vector of u8
        let chunks = entropy_bits.chunks_exact(8);
        let remainder = chunks.remainder();
        let mut entropy: Vec<u8> = chunks.map(|chunk| chunk.load_be::<u8>()).collect();
        if !remainder.is_empty() {
            let mut byte = 0u8;
            let bits = &mut byte.view_bits_mut::<Msb0>()[..remainder.len()];
            bits.copy_from_bitslice(remainder);
            entropy.push(byte);
        }

        // Check length of entropy.
        if entropy.len() != Entropy::SIZE {
            return None;
        }

        // Convert Vec<u8> into Entropy.
        let mut array = [0u8; Entropy::SIZE];
        let bytes = &entropy[..array.len()]; // panics if not enough data
        array.copy_from_slice(bytes);

        let entropy = Entropy(array);

        // Check checksum validity.
        let computed_checksum = if legacy {
            entropy.crc8_checksum()
        } else {
            entropy.sha256_checksum()
        };
        if computed_checksum != checksum {
            return None;
        }

        Some(entropy)
    }

    /// Creates an entropy out of this mnemonic.
    pub fn to_entropy(&self, wordlist: Wordlist) -> Option<Entropy> {
        let bits = self.to_bits(wordlist)?;
        Mnemonic::bits_to_entropy_generic(&bits, false)
    }

    /// Creates an entropy out of this legacy mnemonic.
    pub fn to_entropy_legacy(&self, wordlist: Wordlist) -> Option<Entropy> {
        let bits = self.to_bits(wordlist)?;
        Mnemonic::bits_to_entropy_generic(&bits, true)
    }

    /// Returns the type of this Mnemonic.
    pub fn get_type(&self, wordlist: Wordlist) -> MnemonicType {
        let bits = self.to_bits(wordlist);

        if bits.is_none() {
            return MnemonicType::INVALID;
        }
        let bits = bits.unwrap();

        let is_bip39 = Mnemonic::bits_to_entropy_generic(&bits, false).is_some();
        let is_legacy = Mnemonic::bits_to_entropy_generic(&bits, true).is_some();

        match (is_bip39, is_legacy) {
            (true, false) => MnemonicType::BIP39,
            (false, true) => MnemonicType::LEGACY,
            (true, true) => MnemonicType::UNKNOWN,
            (false, false) => MnemonicType::INVALID,
        }
    }

    /// Returns the seed for this mnemonic.
    pub fn to_seed(&self, password: Option<&str>) -> Result<Vec<u8>, Pbkdf2Error> {
        let mnemonic: String = self.to_string();
        let mnemonic: String = (&mnemonic).nfkd().collect();

        let mut salt: String = "mnemonic".to_string();
        if let Some(pw) = password {
            salt.extend(pw.nfkd());
        }

        compute_pbkdf2_sha512(mnemonic.as_bytes(), salt.as_bytes(), 2048, 64)
    }

    pub fn from_words_unchecked(words: Vec<String>) -> Self {
        Mnemonic { mnemonic: words }
    }

    pub fn as_words(&self) -> Vec<String> {
        self.mnemonic.clone()
    }
}

impl fmt::Display for Mnemonic {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "{}", self.mnemonic.join(" "))
    }
}

impl FromStr for Mnemonic {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Mnemonic {
            mnemonic: s.split(' ').map(ToString::to_string).collect(),
        })
    }
}

impl From<&'static str> for Mnemonic {
    fn from(s: &'static str) -> Self {
        s.parse().unwrap()
    }
}

#[derive(Clone, Ord, PartialOrd, Eq, PartialEq, Debug)]
pub enum MnemonicType {
    INVALID = -2,
    UNKNOWN = -1,
    LEGACY = 0,
    BIP39 = 1,
}
