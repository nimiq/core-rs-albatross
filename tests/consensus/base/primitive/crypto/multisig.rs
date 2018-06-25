use core_rs::consensus::base::primitive::crypto::{PrivateKey,PublicKey,Signature,KeyPair};
use core_rs::consensus::base::primitive::crypto::multisig::{RandomSecret,Commitment,PartialSignature};
use curve25519_dalek::scalar::Scalar;
use curve25519_dalek::edwards::{EdwardsPoint, CompressedEdwardsY};
use hex;

struct StrTestVector {
    priv_keys: &'static[&'static str],
    pub_keys: &'static[&'static str],
    pub_keys_hash: &'static str,
    delinearized_priv_keys: &'static[&'static str],
    delinearized_pub_keys: &'static[&'static str],
    secrets: &'static[&'static str],
    commitments: &'static[&'static str],
    agg_pub_key: &'static str,
    agg_commitment: &'static str,
    partial_signatures: &'static[&'static str],
    agg_signature: &'static str,
    signature: &'static str,
    message: &'static str
}

struct TestVector {
    priv_keys: Vec<PrivateKey>,
    pub_keys: Vec<PublicKey>,
    pub_keys_hash: [u8; 64],
    delinearized_priv_keys: Vec<Scalar>,
    delinearized_pub_keys: Vec<EdwardsPoint>,
    secrets: Vec<RandomSecret>,
    commitments: Vec<Commitment>,
    agg_pub_key: PublicKey,
    agg_commitment: Commitment,
    partial_signatures: Vec<PartialSignature>,
    agg_signature: PartialSignature,
    signature: Signature,
    message: Vec<u8>
}

macro_rules! from_hex {
    ($hex: expr, $len: expr, $call: path) => {
        {
            $call(from_hex!($hex, $len))
        }
    };
    ($hex: expr, $len: expr) => {
        {
            let bytes = hex::decode($hex).unwrap();
            let mut fixed_bytes = [0u8; $len];
            fixed_bytes.copy_from_slice(bytes.as_slice());
            fixed_bytes
        }
    };
}

macro_rules! from_hex_vec {
    ($vec: expr, $len: expr, $call: path) => {
        {
            $vec.iter().map(|item| {
                from_hex!(item, $len, $call)
            }).collect()
        }
    };
    ($vec: expr, $len: expr) => {
        {
            $vec.iter().map(|item| {
                from_hex!(item, $len)
            })
        }
    };
}

impl TestVector {
    fn from_str(v: &StrTestVector) -> TestVector {
        let priv_keys: Vec<PrivateKey> = from_hex_vec!(v.priv_keys, PrivateKey::SIZE, PrivateKey::from_bytes);
        let pub_keys: Vec<PublicKey> = from_hex_vec!(v.pub_keys, PublicKey::SIZE, PublicKey::from_bytes);
        let pub_keys_hash: [u8; 64] = from_hex!(v.pub_keys_hash, 64);
        let delinearized_priv_keys: Vec<Scalar> = from_hex_vec!(v.delinearized_priv_keys, PrivateKey::SIZE, Scalar::from_bytes_mod_order);
        let delinearized_pub_keys: Vec<EdwardsPoint> = from_hex_vec!(v.delinearized_pub_keys, PublicKey::SIZE).map(|arr: [u8; 32]| {
                CompressedEdwardsY(arr).decompress().unwrap()
            }).collect();
        let secrets: Vec<RandomSecret> = from_hex_vec!(v.secrets, RandomSecret::SIZE, RandomSecret::from_bytes);
        let commitments: Vec<Commitment> = from_hex_vec!(v.commitments, Commitment::SIZE).map(|arr: [u8; 32]| {
                Commitment::from_bytes(arr).unwrap()
            }).collect();
        let agg_pub_key: PublicKey = from_hex!(v.agg_pub_key, PublicKey::SIZE, PublicKey::from_bytes);
        let agg_commitment: Commitment = Commitment::from_bytes(from_hex!(v.agg_commitment, Commitment::SIZE)).unwrap();
        let partial_signatures: Vec<PartialSignature> = from_hex_vec!(v.partial_signatures, PartialSignature::SIZE, PartialSignature::from_bytes);
        let agg_signature: PartialSignature = from_hex!(v.agg_signature, PartialSignature::SIZE, PartialSignature::from_bytes);
        let signature: Signature = from_hex!(v.signature, Signature::SIZE, Signature::from_bytes);
        let message: Vec<u8> = v.message.to_string().into_bytes();
        return TestVector {
            priv_keys,
            pub_keys,
            pub_keys_hash,
            delinearized_priv_keys,
            delinearized_pub_keys,
            secrets,
            commitments,
            agg_pub_key,
            agg_commitment,
            partial_signatures,
            agg_signature,
            signature,
            message,
        };
    }
}

const VECTORS: [StrTestVector; 5] = [
    StrTestVector {
        priv_keys: &["f80793b4cb1e165d1a65b5cbc9e7b2efa583de01bc13dd23f7a1d78af4349904", "407fd16dd5e908ea81f755a1fb2591dc7b19c2efcfeb517273aa7c520a5d8c06"],
        pub_keys: &["8695ae0fc477e9c07138caa67d44b45b8d05bee777fd2a3d7c15cec726e4f010", "b14f7b1aea1125029453bc16e31488fec5ec5be75b371003610a6901f4035992"],
        pub_keys_hash: "2f2e0d26bdb53fb8a4735a77251453eb06d6e9db6d67955adc8706336520fec40efdb7c43f396009a3560c04fdea75d810c55f11e8dbea562e0875438957e664",
        delinearized_priv_keys: &["5531dc8d35f336bf3b5aadb03bf2369bb1cb0bdcd588ac6a5d1179370bfdf00a", "f4dd2f8c63d156f471e84af2a1964b03a0b58dafed33c02cc1a44b1b48eb820f"],
        delinearized_pub_keys: &["05750e281698ec0b6d2fb5252b7e474d647a5f4babc8d0befdcd04d5473a1579", "ef512dc2ea4583518ae1c9588d4d3655770f430f824fc36b21053abe7c7cb032"],
        secrets: &["8406d5a4872a124c834e1876ef518203ba343a318557d051d4d5035a82c20501", "33b2d49f68c1091470bf7723253af5a0e755a035f918708203804485b24b9e0d"],
        commitments: &["f3664d2921f41ea96ba8d2ea96f19a5844652bb746e65b2280da39f8a8a4a5fb", "5f5a38feced274ff07a094c271f45cc1eed70abdcc594fcb07999aeaf6c80307"],
        agg_pub_key: "5f6591c8d8a304fbdbb259826f8d88210c3aefce858f2432e91ba7224316861b",
        agg_commitment: "5bf678b9604126cca1148cd3239178e989a91ac5a3fa78c64482c335360bb00b",
        partial_signatures: &["65d2bb099b979b818fbd26428aa5dd54f8e3f1b71c2d3a3934b4338a9fe53708", "01eafa18d112094ff0888b3429b8bf52732adec4a52c327d26d5126a4b36d904"],
        agg_signature: "66bcb6226caaa4d07f46b276b35d9da76b0ed07cc2596cb65a8946f4ea1b110d",
        signature: "5f6591c8d8a304fbdbb259826f8d88210c3aefce858f2432e91ba7224316861b66bcb6226caaa4d07f46b276b35d9da76b0ed07cc2596cb65a8946f4ea1b110d",
        message: ""
    },
    StrTestVector {
        priv_keys: &["af6cccd64c2679d6bdfac26f32ab8c2ad1b875ec3e1ab7933509218bcea69c0a", "459124f418cc9ac5e027886e0bf6591493263b984f3d5d7fdf17867d327b1a0e"],
        pub_keys: &["73903ee860501452abc3dbf31ad484033f11e4ca99ecd99d42fc8eb2b0816b51", "198579bc339d5209fbc4bed6f8dcb6234d30d98b678fc5e0e44c9dfa58ddf73f"],
        pub_keys_hash: "cc99ea463a1b146dcd0b83b95a15d6bfbe10907aaed1850cb5d85187e21ab37242d382f3f431bcdc33a5eb609ff5173a609b446b717e2c08e240e3be293da87a",
        delinearized_priv_keys: &["a577441997f693a0c8463d0abe723d9974ae188c1a98cf33d72152864c5ca20c", "a7a3a56cc6b7e8f502d7317f3f79a41bb6fd06ab0793be5c539f5e44d5306706"],
        delinearized_pub_keys: &["39e5bb4ddaca066b4ce13cdfca93d3b46ccd9341e55e1f8b081a401dbd83dbd8", "314e065b8a2d7399d02a9bfedfc2a9514d87e3aadd08f3d35315662d803d510b"],
        secrets: &["a5ee1a93ce5bea1d20db30f12ec53ad4d3d87605dccb2abceb2dacaf54e8410d", "d4ac2ee84cf32bdad5a96c20590ad56fb4779e375fcd264ce8e9008be79b0b0d"],
        commitments: &["e4c4e8877d96548c49f8d395e4f990eff38108b03281eca62a3430d3500db7c4", "6c3d1be926e02a958717d6bc4934d541582cf1ab8fba19bce0d3c5226ebb69ac"],
        agg_pub_key: "bd7b7414e03273d3e23049586b0a97ed8164651937b17c2e52e2261a89fb4347",
        agg_commitment: "08a2a7e9ed90811e8cabb48ffa2ff1a3b6e4f99bf81a552536cebbfed0a1ef8c",
        partial_signatures: &["0fa57e95bbf4b5530d13a7e4dc405401c478b0e62a1a26319da966d13edaf105", "2d28a1db8447ee6fcb915a4d5a373b0dcdcedf1fab563a63c15c0ccee49e8505"],
        agg_signature: "3ccd1f71403ca4c3d8a4013237788f0e91479006d67060945e06739f2379770b",
        signature: "bd7b7414e03273d3e23049586b0a97ed8164651937b17c2e52e2261a89fb43473ccd1f71403ca4c3d8a4013237788f0e91479006d67060945e06739f2379770b",
        message: "Hello World"
    },
    StrTestVector {
        priv_keys: &["fce1ccbeefe33e3d25dc198f69a58a2c40f24a75d0ea728cef65ad7e309ac20e", "312a4339dfd85e9650e918b3a714196cd2e54b4c84ac6f7abc1db897c2f4aa0c", "726254985d2fd37ac244365465bb52049e60396051daeaff5be858ef2bff2105", "0c817fcffd7c0e90d1957257b10b617e448454a029cdf25bcb8e2fa312c7860d"],
        pub_keys: &["8e16f94ea50415097751606c7fd21e0bb51e340c3ba59f1a51dd430b8a8ef444", "94b8d238bbe70758ec7807d420dcbc0134b65fa9be03ca06592f957118f88010", "d365f216daf3f331ed2a803ec76968e0373f81695c4e8a70776a8f3eeba55a20", "f8bf28fe5d42a9a9dd0fa7a5826cd83a59c12181c366cba5eeefbc0c5b4fdac7"],
        pub_keys_hash: "158910d4ed875025dd26f906dfde891e9a47c9586af60b5deca6ae3be94be6528fae9cab4f083e85dd75aa9dd25d8df6f353d325861163465badfa22683fdc27",
        delinearized_priv_keys: &["53c4fabe7f845a78d56ae8d482d32cae5e35c588824744b9ae095b0a4e6eac09", "821c6db9a9f12f6180d2d45ab421759f0531e1345bba3b0ca686c9e301069c08", "a22bb26d3d0494198676230260dd3dc5cdb5457fca6eab5c38bf3d47fff1ab08", "a83d01c79751c0bb29387c6d5981c7c3eb1861689d317011c839ec04d888cb0b"],
        delinearized_pub_keys: &["33dc44d3384019a18a4acb951faf37d64870ee890663e0018e71845946626e8c", "78b47f19c8b94f5c32fb461bd91c8dcd7025de844fdb777051d0b24de75d51f9", "8e5e398f9ab1a96cd27422967d5930a0dd372ddcc3e5c19b2035ca122c59a21e", "16e8edc1847db6f52a21c7663ed85b11537c4410b8e5f0ff59ca0188dd7eb633"],
        secrets: &["d671a40fabf8735899862b00395c864ac32abee0aa899ced4d9a22b827311802", "d8d568e93fa27da70dae66dde3d7f1b12f2a823e7fa7dffdf0ba3ee65fbd690c", "0886a14851375489b3dd95a0d03d4d6c7783759fab0a74d71b2028e99db5230c", "2413f3d8415c9afdd1b634ce86b2a6dcaad5cbf2fcb33718d020cf1813ec4705"],
        commitments: &["877c7007fc838154cbacef7d4752d55c44400f0359399b3f59be19422b3eb912", "7d1dcbeae1bd00b0ded04ccf76909523535ae2df625711d2461713fe4c113789", "4e61ad68f9c163b89dfad6a17f528d5948fb2da1400386d21bab756e630d2fcf", "773fafeec83cc008fbf63961d98b56612811bb8037b9f3b192738db3fa6e1d29"],
        agg_pub_key: "b57b3ee1fa93230e68def1f56ab9c8e390527618b6f190b592ca40f84a877ad9",
        agg_commitment: "387d7d274e8c48379afc8ccc91e309bbdd6c259588e3f3999efcc3f137f92e76",
        partial_signatures: &["e4b7de85519f7724fd7d00b70edfe3505a71fe720dc9d34618dd77ab3c96e50f", "aa316a69be1fd834fbaa8d76170cb4031adc34ed27ab1640cd44c51aeda55806", "51c3c81056912c1445e9b977935233d55aef075187088107c2ce9f33fc00dc07", "90d28225e5d752b5e02f60d7ac461ee1d4ee1c713ffc3d1c43657cb5c1031008"],
        agg_signature: "95d7a86b1662aa727108b936a9902be1a32b5822fc78a9aaea5559afe7402a06",
        signature: "b57b3ee1fa93230e68def1f56ab9c8e390527618b6f190b592ca40f84a877ad995d7a86b1662aa727108b936a9902be1a32b5822fc78a9aaea5559afe7402a06",
        message: ""
    },
    StrTestVector {
        priv_keys: &["65edb8c173fdbaf5e106ca53069cde47c2a7627518228d8269a3da35f5fd0001", "574e47b97d5918ee12f3872c7843b789c34298ff7cc59ea049586afe63f7c20a", "a8177dccd4557044db70f3066960a1df283ba5f0b8ffa997c6411c9119ac160d", "21ea275ae38602ef65aac6774db8ed2e6164923b14a7e11c40df874f453b780a"],
        pub_keys: &["3d3f83dc9bacfc90f64b520099e97a81e90ab58a1e10b5c95631c07839c027d7", "c5539285ec6a3cca991eb8ce3d0f56311aefa706645db278b6a243fac3ba36ff", "d60bcfbce4d869b7de3a6efa2ce12565e77233eb091a34bcee81e6981ddf897b", "43b51bc6464d60d95ea36f173f62e23f5405160790aa925694ccfba01d834f18"],
        pub_keys_hash: "2ce18109c27ef0f1112b59e1decbd25b3cfc0347112bc75b053637e11021da8d5e0512e82b64ccd8054da021973f45277e275a0fa6bd89f9bbef8015462c3471",
        delinearized_priv_keys: &["f2bdeb36deadcd5d2f46dd8e72e1bc99a06e305c758a228ef499351220f86c01", "ecdd4913aa1bfeb22d22b1c91b3945eb4ce3018bb7f900ba2bee6fefd6b44b03", "36b71860698c9efbb302c88bae15824be1a0ceabaa7b0c6212851321be6e5e08", "a2949fdc45b8b723c523540964df3d3782139e0dcb65c8145d2556cf5490a202"],
        delinearized_pub_keys: &["48ecd45dcef9a84152cf5aa7ed7f2c884aaafab4a8c7d18b0074169ffc442f1f", "39a191c2c6eabb8b0485ea00fe5b7da022a4fa95a8c62c9e51421338f33dea34", "9b9e0b154168d124d35eda474592119e6425bceb90be32d0cc9961c9ec9f1597", "25c98cb4a563c88c9ac4945e1ba0e83d2ecf6b7624a8f94c9f8a8f3640156079"],
        secrets: &["d8bf664734b0fe7ad84dacd52b0c1aab7b42b1c87de456e666b0473446cee50d", "df62a5936edf38fdebf1b77c0e88b67a3c3049c20e9b82042d18e799d9147f", "62f9d569b6a4923a282d6ae5459f4fb9a75a1ac237906198edeb7d71e174b50e", "203b3e15c22c24279abaadbb321e2b4d39e1c91183fa20698309f5f1f6154903"],
        commitments: &["b3a8765c83f09db199140b9f877d116b7f70d464004cf7a856f089043ed5623c", "efb62e6a34cc50f54118cc4e3a5123cafe440af4245060c7a8bfe0e9de99dac2", "1e1d1dfff9ad5e360df85e4ca9496b51fbfc0696a67cfd9e6f6b34a07e3c69e5", "74b01bb35dbed39588e3810635b709ecd4547e1f9c4376d5ce3ee5416bbf5e38"],
        agg_pub_key: "640d9c1a5e918fed8e28db6c0d3270ff1c45fed558a00215ad42326c77d1b7f8",
        agg_commitment: "a4f7798ee2e1adf96053b443cc3e2c42d94b15e3fec5ae8637c05b71cc90f7b6",
        partial_signatures: &["9ca3d1ebab436b3309cd37be7a81f3778f1831ae91ee446ae056d1baa8e30b0a", "1fcc5bca7d16245d34a6b363586f20f7280eacf42b9ddeeaa664501fd1a09a0f", "03e7274c1c8a2a2aeb306a1331a26431c8a80ec84bd2717eb6a4e6b66f4c090d", "9800eb815f4df822198970b032871ec364a6951cd47f5e49e0bc0819968e8102"],
        agg_signature: "7caf54ca706b8d2d95f3d69f7926d939e5758187ddddf31c1e1d11aa7f5f3109",
        signature: "640d9c1a5e918fed8e28db6c0d3270ff1c45fed558a00215ad42326c77d1b7f87caf54ca706b8d2d95f3d69f7926d939e5758187ddddf31c1e1d11aa7f5f3109",
        message: "Hello World"
    },
    StrTestVector {
        priv_keys: &["223dcd7ada5dd16cf4d161b293721ebb732f0f5821f2a555bbe6def499364707", "283f4c6a4674791e0e24c97c2ec958f3fa87ab42b085824d475b16ca49c35604", "6a828f4a5c6de5f4f6ccd846549d2eb6c961e950a1fe32523ea7bdbe28291a07", "1f89b89ce7e31335fb7080756b1ed5e6dcf947250b1d13e7c8c6713c38b98801", "6ad88c8c37d8f054fbad5af4f17ebc24ce2bd7eda6bd543f59f3ee70601a7f06"],
        pub_keys: &["8870e57b18faa37111e971b32c844e2bdbf773179f9103057b43478aa039ef7a", "12d92aa661c24dae4ee63e4273fd5ca9ccffdc746e6f8cd72b2458360f957562", "1b6b9187d22d56baf6191770a844da0c3286aa9df9baaf02c3bf94f14a833f61", "b7248fbcdbc09ac7fa8af4971ac2f9e77a9743a0ebb344c6499e76c498fae579", "6239f2e0f51221d11c8a02f34f15f0cfcf64155bd33817c9c794dabd925dbcea"],
        pub_keys_hash: "b795f34048fc0fcd8ee7d7dc2a48f45a66e6cf7a0b800d912f0a685c136a4d826ea0c6412a6428db876b398abb7b52931ba3102a85ad81b0ec7f547c4272ce9b",
        delinearized_priv_keys: &["730f544f56b4280a92212c20deeb3bb0e24e44ee6f98b13102db052121d5a104", "d779e3b4a43a9ce9edf4fc4781c9200b23789b8aec9c9ff84cc012ce93821b0d", "a330bc92ebc6fa244b11a368cfd79fbafb7da370a58679773437be9bab8cc903", "587171c14d9adead708dc0b003806bcac0c07d3277ac4ca373b43e4aff32130e", "7be213bf031e0f8bc9c1ea57d05c430363eddef6af79f8bd165069850941c00d"],
        delinearized_pub_keys: &["335253f4754ffbc52013d4cd383f84fd9359c52f7a15fc7d49f8def5e98d36b4", "9cf9986122407edccb94cccd1ef20ec5fbbdae9c66a1e9dd51e5838aa4099013", "4f6b692774205dbcddda8d6e4456c51fda5eb722ba19b7929df213ef24fb8974", "f0fe119bac035c25edada3ec6aa756ba79a4668caf652084e8ddf4d90e0d451d", "6f0da8e2730d3ca10340c2e935a388ee6e90a879db62cbb0c278ac538a40d437"],
        secrets: &["31313231623461663039666262653365353739623235336234666236636538616639316430396264623630306536316435636137653137316333383863373039", "66656264353737356237353030636332346230616334373961663661366630383837376634626261643438313836656437343164623433613030663863623033", "32663838356565626338356263376633316632313131316134346333656164363164373639643061356436303165653039653032653238393030306530353032", "33383266613838623939623236353465313863363065643564306365373963613636623563336562366464346130303539653063636166626464383863623035", "63323166653561373530316461343932653734326366356634626461396562336636326230383166636663363232393732636431376539393336366333643061"],
        commitments: &["5cadf09a490b4187e7189caf199ac88498f9b1f5d630cc9a168384cf8c4d5ff0", "91364d6bb88a556909dc6f63dc78e596fb1845b2306527a8d4d3a5125e51573f", "719199500c32a37750b3995576106bc6e606a6227716fa1eca785f56a89601bd", "3da1e1a3f51446fec43c7eff73b4d95363ad53cfa01b055f6351650f312326c6", "6239237ea2665c0c1e2272a1f88538004e6c81b7f2ae12a3dd6129f526b79bd6"],
        agg_pub_key: "fb47050dbf537b8b383884a743ea36002bf74b54b358c17b039d4a1cb6344316",
        agg_commitment: "0bf44a403c18cb317875afe02c2b19ffc2a16980a06190951e2127ebf98d327e",
        partial_signatures: &["118871813ed4f1f4ff754990a0995b89abf6f37d4a8dfc14bee256f2b7710106", "0aab00cd491e6120e05405b0f2ea75000ab079bc4382b1ffd822ae4125430302", "df23e6edd6ce6f12dbb3e9954fcbc276090ebc360d317680b251f331e2313802", "b4654c37c22d32fa9a7424624794575c5cb72a83306321d406cd0bb66c275d03", "f7fbc01438cd0f5e1a372ed333760b65a30f4cc7243913b5a44d80800d51e706"],
        agg_signature: "b8e46f2b3f59f2279a8d93687f6018adbe7ba0bbf0dc581ef571849c395f8104",
        signature: "fb47050dbf537b8b383884a743ea36002bf74b54b358c17b039d4a1cb6344316b8e46f2b3f59f2279a8d93687f6018adbe7ba0bbf0dc581ef571849c395f8104",
        message: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum."
    }
];


