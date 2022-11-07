use std::cmp;

use ark_mnt6_753::G1Projective;
use ark_serialize::CanonicalDeserialize;
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};

use nimiq_bls::{
    pedersen::{pedersen_generators, pedersen_hash},
    utils::*,
};

use crate::serialize::serialize_g1_mnt6;

const PEDERSEN_GENERATORS: [&str; 50] = ["12c7fcf3be5d31db7639859edd82cdb04b92cd3bccbc9dd5a70e453b00b5c018081c4bd71b0b26a62a7dd9e8a135fda20c9e235df12b1857a854775e61c3ddfef5c0d8714c8f262bdb1f7814e1b0bb4559f7bb0c3e2b5b758a051047a7ec00488e1a317180515b051c6a27d8e0a549f201f04b2d2ffbff852fd14e2932187a13e0f400bfbf73c32984b38cd2c2f2341ce80b632cca4eee382afac728fb5afeff0e8c80a09148206c3c26a8d256c5ab75d723bb6f5ebb4c653e567298dd00", "7eafba96cb01c39de48f47e937225542e27cd20bcbf4a359d164916d0b03697debcd64da679adabe49f3850400d4c72f8495cc80bdff58c795360627f0cc31c030b2bf9410207faee5d2bc3caa8d33898f838e34cd40a3406b6845f35b4f0034fee543a3ff306069607f996430ccae45e1801ca00bc955f90df73dd828c20f063eb14d18b548844ed5b29bb32276e3a963825376458925154eecd99ae44d7c128651e8c339f3088b39fc019c52bd11a7501da24f6db74d3429c5be48f100", "5e9ad1ae3e285b1c08eb33c0b6d016adf814ae4d07ded7b5e8972947bba7f1a4fd0aabbe0fc65672e8fd96a033df472ace4e39ab65d2db6c01b9e915e0b44bf8d791b258202a958de58b55f71b426cba233dc46f63975c0724b4e0bf76ed000684b70a78626e65b6ade6efcfa281b8711e7d335d43de04c6bff247504097670f25b879f5bddf21b965e300030de80066954a10a1d5b183272b41b3677875d919664d68e1f0b44aab16323119074f17405332940264109b6d474b3a3af700", "53c1de71cb92f3b7edd09089008555aa8f9635aad48fc6e15afe2daa28a6976718ccc932c1705ef4692567da47625f422d00fff5d1c569dbf306eb70cb2f2d77537d4ec30eb9896ea0caf14a819dbb5a9170a14d652c559a853c9c14c314005079887f60b8bfe14521efdf4301aa9d5db158132b59603f529ca7ea0beb1021cee56b19c392461fcab4b0db88d06624af35be1131114e71f4b510cc3179d2c613f57e972bff7cf9461cf87f91cf5f9c9ac73c899e3e349b8fad85428d7600", "8c9c06e872c1fea026f2c8d725d0cf0cec0dd588ff6c27ac75329b5a7e166dd00fee64398101c217a0e84013f7dd8837d900d3e4c8ca569299ec0520dd794f5eb5fe19b53881e1c9a8079ec9c2bac05ca2fcda86bfa14a4d3b157ba7e8f3003c46eb6466ad0fd7b8b2c4c4aece5dad1d66ed15ae425d833948c9f5969b907911d0c8d4c6e1422c4b36aac113df9aee2b202246512659d505922f77b505a5e6d058cdeab1069df4549500a1c975c18b885b7760f0df8f73e7e1b03a149c01", "419a022bff3b073244fdaf1c8bfadc1af704c87cd30e4c7e43f2d092c581dfa85992d442cd7312f41f57bf32965d4369a87e99229fef9da06cd7826011f07cc37e3e43c3c4cf62d6311b11a569191b6a0d2071e0d4becb6d9c87d40abdca0003f57c10e501d5c9a8f39996cbc2f56dbf173b547185a7ab8c2a43c502f2e2cbda496eba5365b0c9c80481c133e3a9b3d3afdd5ebf8fee40c82be2bab176c1387ae3c2d381d10d1542e4f93d03546a8ea4c1903c54a4583634a9ba4eab8f00", "b74c92d9756ea096f7bf3f015ac119f54a2ab45973eee48299c2d68192663309ef3b96f77278d644776ec5b73f7d6e274138279030c4a7b22f7f90b973c43353441e462d4c6af6a9fdec9794c75b437bc3433558f4ae155c9b661ed0e51e00ff19a283f4f9ea08eae1cbbecbc415574ffec5629195d89324fa9e59952c59a6153468c5ee0d9b0b3239b0908ed5c0e597dfc299a14a237a3f2ff591f716f63b1f7928b5f4ef9b1ccbeb1724b9acf16810218bc7162223b97bdf25357d2e01", "c5092e61c200a911f326ee62b37dc1fb7ab4e68a994085aca9ef70bda8af495867e8e37cb3c163e8f3780daacb9d9e08bb9daa883822280592c65b7319ddfeba53efbc6231906a2c3f9ca1f3cabeb3d231cf92d91bfd7c2b658ab3f5839b009246b9a4bfc3213dd8ba3029b26e3338228732b8d91e11d463183d5a9f968d076fbe01dfb9f095ab0190b28ca3511fe51068765e85446d083ac05a6ffeefb58e596eb1a7a2fcfbac1ff39bd7f005afb3e8659284f9092164ef27d745761f00", "fbf7d78d8e0d4755985e16b839b296cfbcc43fd252d709332fa2d70db05c80bab4f5ec7fef207d1f430f58f8097b030209f1c851914d43a4209e5344a5c867ab69690d0cb972dc6124709cb01aa9aaf174237d3ffbd3b78cb3855d0d769600633664d241a08edfef547e6aa861638dbf417e18e13ccd7edbf4cb0259a80935981a3d99191399caeda5e9bef91405b993e3ff6fe0d8857e8ad460a2fbb6664275400968100143509b2263e66bd5a069314fa21925f039467548ae09399a00", "536fa69b8f02301a2a36865290591548cfe625e96e2b21411dbce29d81804661012d5918f27f0ad5f528b26bf4a50415c6b2993bf90fc58750f12b700ef76047a90a98188d25ee632d0c82b8d437cb009015bbfd4faf7e325063d31bc51600373fc0d14da8c11135e9d7e9edde20152a7ff7002f2821ab58feb92c7f32d400896c34e4eae7412a3a705daecc36f3ac88e103246d3c0455a4b10b9fc35c2b0946f9c9909914d760cd910abdfedec0e7f2d4ff02807a28b903d9916af35e01", "39897294dc969f45438e03493c91b3637e65bd0cd223489b4fd707ee123d6f29078743308a2f11f7160805312486f686b4f3ee2a630b0904a6aa1043630a1f3b01bb2f68060e4a063cf7e9c6b8116b92f17ccd4d7dff22c06492170449da002265337685f6389aa3a2467c681e8dbf71a5bb5c440a6b5a8a593c98cec442d684b2b5e3635c0436046d74c902acfb8413771b11258e57c0d519500145efa569c969244ee7ba9c161e2da7e6d98cef1b7951776f7e9db0983e0407cdacb200", "2320646f7966d362395b0e9513b7ab9f1c5e5be03f98f479ad39883036da7b001f019430cad3ceb970f3ab593b117d30e0d930f2ba63d32cd979b61b675c00db781ff92818e3e00aa95da0b63b8122a22965ec938592fbb922d686856b0d00f7e581d3f31befb5aa2c13d5a634a948ab7c07818341bd28e283f62004d9cf9139ec20324ffdf97c7a44f0f8065ae70e5d957831444c829aa09416bd8f515b8bf04b3c91c9882be797d7a26c98f9c160fc6e365ca13c572b8b40b6df597201", "96451f57aa3a38736b1365ab8e0d795c3454e9d4c9ffffc8378c85b9caae84b9d45dededf636cee4b8ffe49673c25819ec2629ff769fd77fdad352569e8ee5cd47afc4076abc8605e8c98768e6ce13d9d5865e24b92268c311d6cd3d67f2003cd0101f6924c92f86b5df7c9cc5c30da617104d2e5ba8e20de7a8c5af1dfc9824100bfd76a72ebaa423ecb87beea7cbb01666b880c85b4f0b048412e6b86b89ef80873e10c297ad7b36cf0d0ebb3c7149db1db9da902bb5f0bee097860700", "18579e7b1ceb21d49d8db2d335b0bd7c1a882343b62b12c2af0c92ce1ff555080ccdc3fcff51188ba23703ebbbdc3b47ae8f3120531a8a6d827c02494fcb1b735ea2b9dde6d8d78884601d6b77f16bb1c713d344cc886e1e3f7fe9f588770051ce49ba106e837b1374c6906c23ba0f8c0330b2966a00fa19ede5a9190fc99136771a13fa04439afbdc3b8d0ca7c903d2a147ea12611cfc90f791b0055de625d47c9ec43a5f5b10ba4c61038867b21f9c3262b2ee2ffeb88fee14f033be01", "aa27377d32b52668dd3051410ad68f2e2108bc4caf7e55481e635df24f6be56459f9aeb4ee86ff41c6ab3bf3c575e0a4c4f1c9d69c02469abaf9d9e56d1524f79925562765aee24f3fb9d14db348db20113153a3718e8459e5b2cbf3a0a900b30b634f2bc2437c8f0e4cd6f9dcbe97cb94fcb3e9f8c199839bdfe0eb8bc42abb710fe44cb5c3f0c397bfbfb47f65338cc8246e2c0fa63fe44ba4b896b47218db7173c35a8841bcf9e24633b5e8dd34d030ef1aa2c0167d90f26260bad700", "97ea9500bdc79dc9f631a137c38267b45168a4be685b978b2174cc408726ef3084da2b5e48fd6ff964d4adee696f76ad36297486e628d73ea71fa1680d6060743d654d31cf9e1627b1df2f56d48905fb2acf5e61a3a6d56a53978ff497c30066d0b11ddb7ad46476b36798a9424133fea98cb590babc004f7bb56ecb54cb254a31e837b84d8475599791a5bf9fe090a57b5e382bf29f804aafda44c7b3d4f540e9cf2a436aaeb56022b1f16d7c4f4573ceee0a169dea9529ec0eb9d28401", "2e22a03715d93580ff4a9c3d54dcab135470996346e1c8d59e13452f9717df5c9c1fa27acd4c4e250dbf787626032df3177072e147491c9a5a4ee4af3e97578ce04b2a7204487fbe777f9958edf34e01c617ec3a374136cf05487cd0f39a006b55b8ba14d548cce0889a7fe77cc7d4f758183c8cccea883048d930d6e38752e946d0279bcfbdc3e8410826988fc0da07c3d9ac754f4562472e08e708d80a28ec0711fa97a5ceed2254b2042585a67fcb05387cdce50a1699928e31127f00", "875d79271202b504678dc4bb8418df4c9600f88793bc588d72b7c0e6446142ff82151398e5a71ad718486191cc76be4cd1b8e3345b84ee78bba781e76f65c28a87e7583e48a9b50a5b92c8518780ea017f8bcd446744a8c0b4b7274a95430086cef137f26f57a91b3505de91c89594da2002476c492e93aab5ef0dc9a4d3dfac4f7f4e7cbae91cd48b395efa638c6217cc3422694bddd817aba504f420946ea52841681d3b50c1994e7abb2590c5dc741295b340b7dc04a9f39e3c5b8601", "d87d0891d76815f9a8e2386942b6f682cace21b6560e96d2e96597d512efd816d476d178012648dfdbf5c3847007cf973a9ab21c69704f1880c587862480899562ade85a2135f8ca1c9b95c4241cc3990a9d759d3debeaeb1aa4ded3b39e00177eb743bb691cb307a4fa1a7c29bf059b297a6a36120526af05cd42bf224a10fce54739a47f8ea1099181a04e17ed03c8a7422e9017aacd1859a7625a785a9df374c54914e55f1b653fd42a629d444d53eec973af77e0e6552ded81695400", "8144e570c7d64ec77b82d8e22586a89bb8b28bafd6d27e0e9f3aa0a3ce554256e2e92166819075cf44adbe3ccfb438778890a6ab8694c104581c6f7e6e37fdd7830f5bd0d96c893f78f950c22fc1aadd3b703c2712fdfafd468dfc8f7e6800d9c3e031e93b50225d717d876fad9d7b79a7c36d0774e8f1a04fd51c2672023b50f0d80a531c4fa2d0c8d4d9b35e27bacf9d4314a2d9cff2c4c49679b82764418b47342b84c2ae2e8c1225004a47d8cdf068f97ee14ea2b92c1b029703ce00", "91b72e5aead35bf8a57a862b4fd224470070a5cd37d1b93d1d90566ae1b5ea7853e4ca1586fe69b98568848f751b257bf591b6bf209268f6b1669110fc13a60dce6ce17ecedb82f03b8c7ce5a8560818670141adecd91e3385c51916eb7800204a0ce8a7f459317388076a85107e512f5aa7e7c725adfca87ddddace4cd94de104f055ee0b0f7fa008ec1069eadb83c5455cd1a20a0e0247000390723799f1cb599e5a2880962cb6b796c0971bef1bc34e131f647f68e572a7e192d20901", "b0907cd4f5b5a57a547a14dd3abea05da6c291ad4aa0a816c05e59c0ac95a37ef8f447df60fd3811bad79b5a51758da9bd335d55c80da8dec43bed84b1b7364bc8c1617c68ae66aea3896a86f5a0b1d15d24e537c1322ed0a3f9b075919600284099cc565be2d6d618f2383211a7daf94b40045f51843361ea73f403584a853840c1bc8b91cab664df2bb23be819072be089a7fc56278c3ad82ec679cfdc2301b9ea10f1bc2be7dca75ec80819d6323c3a326be282ac9d740ccfe0bd7301", "c44f1509879f4555bb6c75b8eb01cb656fdb284b7339456789dcd6d981c730c38d35c4ff3778318d3959026828d277eed83054c6ee4b088003eb1dda0f66f5bc1539003eae2f01defbd562d907aee75a6c933e4eb449a131f06e67c019df003f9e6f949307c4cd9e87dca19ae1dcf94b5873e287b208c11975b219f9c42b262d3c4ff608ca06199b2b2bbfe39a53279cafab2d9dd42a23402907fd043d3a5d5ec365dcfa9b8ecc8720a8581b4bdb29380b0d7a2c49bfdecdaec2722da900", "69313602861d08fd89a2c10cb35523342a423283a045f1c4eda8746b26202679735785434416aaeca0292ba6a3302cd6362b4a9424c6216fa5fd5bbdcd9b0cd229040fc42cb07e14830738c15e6a7d357621771322e23f9f8b26967c1b6c00060edb6032804545bd6d4e46f0e25a3b1ab8ecc23aa38846648e58d9ea727cc390eeb7ca7c2c4d34ebb158753e79df689a2a88ee44240542958c643951a47ea4c68c119466e7b66b20148e89a63a085d6d618c9d53b1aa77a2b63e34944400", "6d42bf95a174eb7d441e3ea5fadb430e2271105091e593ee6857c1cf837319aab4edcdaf64692a8ef19adf33b6bce34013cc15a28edb68e541e2f121b8c16a94c7e51bcb5747ef493aeba653003e98b9c0ce8a01b93b7600dc7420769873003d0663178ee27e3561f8a5cd20733a285528af53f83adb3a91a7b5d5d4e3b3bb060fb5243ba5a76bc8d9fc09fdb088376a9e7bed6445da371cde56ac453fced7a09b69606a48692ee36b2ee1d8d842af8273715671440b66c21a11d2950b00", "4e29d13e1be529ce648c6bf95e881336d567936c856d7013115721964e40685a6b382f80b56f0f2058d0b98a86f1dcb7e52818fd39a4bf6400739d991dbd5f0c9590e0ed69283671f30bd90820f9995bde137ce42e998f5d7a3a91ac12ee00053adf93f2c1012fcbb13fb0f2f78e94088b33ffd563971ef94839026b251d5ef5005db88c2faed00f8f2200ab8803825a1d99460e854519aff03245d1a575218754272c231cd4a1d7a5efc9928e54fad3c4a9cfa399b0f623565e1cbd6501", "7be10a9cff0ec4248cfbe8f17224ea48be873fc137367bb1a5d543855d18f51636b877cf132e98a3c446858e0405fada04b2eae11bf2f823cc6d30b29858d56ddb24350ab88a9a9eff50035e8959d3df9118b57fcbbef38970d3a1dd2b7000b20e8d3f330e34bf490edb1698d5212bc052ac1b0b5cfcd9e5f8ac1197521e7c728fd13c84c97136573fca24722c08325df9b3ed6566ca300ff1d4d959c4e3f993140270441f2a41eb1c0f54d40cb2dc2b5b2a46b124d82c31ebf439f48601", "cd68ca94c0ff718900c8027af3b904816826d557846afaeca2752f2f68b3611bdcd7b32481c67fa82ed51bcf37330a9806184dbd922b7a6f35bf3b8b4b741bb8bdac9362ab6ce48dcf4cb02fc1e0ef94a2444b3072ca09fa6a22fb87384300697c487d3eff57eace3209984410b4dc568c2d1d8cc2f4cdbb54c456776a29ca898f6e3f8a9f7e27b5e82f761f81c2c567454ce6ffb5ab760c69b74b548f48bf841e48ea32339434e4eb7be5ef5bb1ecdb8137562407c80fc8f5bf3736de00", "681b5370077c6fa8c53324f9d12eb3e25fbd10343d08446d3811c644ae16e9e08d8c1f29b911c2cf0001432a8d1839f4d242a55023c9ef57b03ebefaaa7b946c4abf914e60e0d801bc3c0241f6fc700b124f4c41a0c6a217f3bb519b6d47004ed8c770e99ee465b93d73b4fd458a21cf1406bd0da356033016f9a2a2801370436b6d3749b0332350620aa1365405498baca8f8fa46f36344b51581209ebbd912b44eb160e322ea438ad49444200d164d09dc6e8f8114164f2e6f488a5200", "38d068ff1e0d64c98510b489e0c0b2ea1e1b1b90bcc9716c46711473912511bd03c1bb2ad92fc8edb2170fea71548bb24266fd203fb91eb79f2f5504502a294d0925162019b96d7ffcbab5ede237a80615a3458c4f5efb2e56b2e133c9c800a029f95cbd2b66958e58f1d817e4062841c7da367a1183d6e44fc58fe47004005f2fccc759e1b65bafe30d8577037a5c6e40d0a6432334d7fdf41e4bf0bf8a53b8171b2c34aa3ff9660f4ec9bf6db812a666ff8464031b4337cbd9a69b4501", "e17d140f1854d4bc46c13d61a844a48d431f515ecec7ffcd63bc24521baa158bae6b5a7b904a15605e9b44388667624099eaf0ceb2abeffd25af3ab734bd2436c17d453e524cd1eca1f45e9b48900d43fc8ca5e695658c42b49a3e84b99e00e5b10d71a41e41421cce5770a22005fcae256be4eac88327f6a9b15ee77964f4db661a9e0b07dcb3faf611e0540399bd7c5f57a4bab208b9439a671edaa3d09e2bc0632fc1ca56c9dcef4c1bb03de19c5e6bb80c55ed0f148d110d392a8a00", "2be7a18bd621704181ccc49db4aa8e5c5ac670006abee7d91c6a19fcc2de966afd3f3ccddb15cb728d962287d4ed9ebf606696c4b5b75765af75e8f26c0dd13c8b2f23c23df67b46e1204dd283b3e0f6d22cc8e85993ff81322311c71e06000b265f0eb9810526560a30addaa6cfaea60fe6e7b7ca3f6ae7aa3eed4c4fcf1ff25e52d4ff42d6a4f982789cfeb25346483a34a69f2fa95f8620cb9cf3cb056427080dd5cdedbd88c45a5b0ca6bcb11b4bb989f2fb0efafee0cc4590db4400", "af5d3ada0bfb272ff079180d247d69c58963a0ca580c6ff241bf762ea8d0ed552c5e331d91da148bcd2d6ab63e6b816b3ee0aa1e8b2d9775c3b003f8b820dcfbe7b438608a8ff9aa2c8f8dc9887452c4b4c6c59f9971991cf81e3ab0508100a77725c3bb6c3f984efdfdb3afc15d5c57b316dcc308adacbf2db7ed92e21606c1d6892e2f6d1a6e4543bf024832cc91551a97018cc654b7674b2fd6ffeeb3035f2fc5eba4204ca697169f39c5de1af54f94b164fe9d8cfe3f6e55461c7c01", "9cb465f1ca660cbddacf142a71d371198772af50ee7f8029be56fffbaeed07644db5bdda42281507bd2f05ecf77747a8c19f4cd0829192fa5b11d34ddf6435767edb1bea2b276daa51b94e7e71e93da54c86cc5f4b1930b78cd302e4238400c04e6879c4dc8a1c2bb3072bb0bd976b16577fce0e235d1dc12a0c8741c0fb2b2182671e77723f1ba0a35cad8d3f92dae918a673b58c1c93d21ddec50ece2c6fa5d235a1da16ec8a21cbe214670134dc0ae4d702b6673962fa1ac7047a7601", "d1f04e24169cab6b17aa23c30d6af3de5fb568f79286b01f9b400a7d6d39080c1c7f9760280d9b281df1d8635e7a87dcf379b3c72305cba0b0f90352a9ca89cb905ce1a9f71ac92a8f0c94ede1fa3553178fa4d7dcbedcf3a03d2297758000cd73aebc96034f1fe489c2c8c908d8a703d57b40804882c7def3117eb80bcdec709c9fed96c50c833869c8349e430d3925730df9649af4a882c18e3e6e362c4cf411a4bb31aa4c6c83144685839bcc3c31f593b7484775510a5f1ff8ac9d01", "893bce81447556d0872a55190f6dc76a2354c48ad8911b1ce1beaac3d1af41501830d206f8e4a49634a019d2579e40ef96e5606f3cf3afb7d5dc5e0a451e3c35d975bea0a29be5ad6c0d7ce94df7e191bcf9fd601f62a7c336504b6ac05a00f0b804e8d9a918f436eb98e1c6c25a1402129219c3a15a894a2f1619ad6c2bff30aedfb2a2939c2ce6df367f54f4706d44c4ffc64c2cefe51a21aa7953fc449bbf57e058cdca42a714ce8b57525b3fd475944c5bf4d06bf6ae72326fc53501", "36e1e92fdb26e08e3fba4f16b43d559d7c4a717ac7644600aaea68df6f678cb8d318fbebe232ffde8a59ba2f7b85a87e8f8490f09caa625d3fbf2ef32f2adfe83437e5ce3bec7f2dc894068a7c771c2a5a1388a1ef726d5c93d93925fad8002bacaaeac01afae81cc4cb891881003903b3c6334aef2812b504b7360d25d76bc4cca4fc62584faaba7dfa7e90ede78ff67491d8a95c176aeebf8a971815ba849c3498cfe4a8b8fb7a558d50496aad08f115b48657379a80b09579655bee00", "40f8f8490b4f389b6d259877c8226f3e73512c87af54acee5436308319e85d174053d93aaf0b4341ac5700c955c4044c1445d2ec40d3ae92664839a1b80b70a7ede8370f5f38e8cb47c51b21df31d71fbc54366eb938873589d9b275381100d678196c7d724efdb9b4ffbeca0a328d4f77fb242cfe4a43b65526e9480c9537305c2c936e53fb587cf0aa5382b63a10e8c9a7b68a0d2b3d8eab3fd20bf238af6c3d2accdfb40df0b61a7a8ada840fed0bae04e1a3bd9ee6dc72efd81e7c01", "cb12889489b3ead9441d44b403ff3a61a577827d523440352d92f4b7e58e2e4d7f771bad0a541bc9051ff79f2f230e8d77e285ba654a03dc5a0b954168a6fdb2648e65030403d2beea85d5130961b4b8991e18178b243e3fbf80ae1febe6009353ea8030cb7d667c9799c420edb5c0da396c93f95e91da31e814070cd06436101fd4369ca232cf7da1f28fe598846a23b725dcf6b3cb920e35358f1771a8f79edd0cc04a9b4f1e4e7d0263b7065a6f420dc59550e42c01abc9922b7c6501", "206d12b6c9ad1032d0e1b7f2d2e64a609380af71e484ff9fd36ef92c3f7683ad4b0f05caf95c96b956b033d86225ff5cae3efc35c17dc14100c529a563474a74db7fb70a1c1fedfdc6708a6b23871669b5b372a41cd2ca502097e58d0c6c0098d92053ac1928049c25e81472b8872665cae230b8dfbc2aebf2d27e143afc80110e1f0286d40c338e1d381ed2658e2c187fa3d93f5b8a0011d010089c9826a7e8c988eb8f3d2c414503af756938884f40a30454076c676711a70be2ffb800", "04243e1a12c0fadc94475a118e5eeeddd9bfb46ffbc535600c9a5280b4fdfba71ac73d624e2c48916fb870982a24db0a59094f2a07db07d6c8adfb14009ca15be2920dd96e8f90cd4290b4375482041135045ac3d945dae7c131b2fbc9d000d2f5d72266b5ea31aa28d270728368cf0b46b7dcd633fbe0ff520e305a3179b74b95b99e2243dd90ac5120c349bdfb0e9c8a8daf33b65d769e2a16affbb91c093e4a67920a6acea48aebf07db10590335fa36d7ab753469aad96c08e218e01", "65258bdb52d260d20f54ebc7df46bf4ca3d468527c9748abae4d38e07fce350c12611839430b38aa14662ed701619b65993a2c8a207193303dcf036724801f51fa83da4347e5034ae18043c074351e915165fa00064d966c2fbe610d7d1100529ce8a64c2b97df389aed3ccb7c12c95a956f0b6ac5cc2eef8af4542dfd413b44a40813311baa01cc0adb4e19468b510bec1cb5abb320e39b474af18ebf11dd4c547e787596fb5cb4324e5f86447044334f4fc1162da7c7e0f566cf244001", "559f7fadca8b6209918a89c25f6bc20bfffd3fbf24b813ce6ca4166d7e25059dce799ec995fa5235c2a757fbdc579636f5ced3cfc20f8f87de8c70af198a5ae7ddfa263cfd088bb1058b5d93f8445abc594bb1e18ce12ed8243f65aa94640071fe2412582eb27b0d2d7528d24e6a05d0f7cd30da21bc22c996ad0cfce4e2593ee59bf8848baf3df685ed15e6622c1a33c483a5711ec6827bea8d1d045726c0d8f67b860d8646b82d85f5384f0daa73a36e8979ecca46d3a86cf96f1a1400", "d907805ee521482dafd51dd398e0cd72ad9913b32ce6d2459619b28dc47c2c396a5f33e291b6d9adf50e34c7f0ddfd486b496b944cd0de79a47776e93fab0656cd2c834cadaa2e70fe6b4d5984442afd34dfc53af03a78ce2236b0611210003d0ecdf629561b868072933364f6e11403b20da9a51f818799e2a67732f04f7d5783f0039663f6ed329e7dac22c45f55f7317a61d90374db9a66d582d79c5060f21e10246a9c9b974a91578d848dabcc4d0a252e7302f85b74bfc3969e8e01", "d0fe1de788752567a6b4386ebb6a3c27b5c63ee544a1d6ed31639e78aabc9bb2f137f11b06e404268940832d900d3453087cd8143430511ff3f6cca26ba2528eaefbac93bdf012d6881696167839f8e1959568e701a9b4eba0568141a9200086faf7f5780766bd115329f3ec3b03c04648aa66549981b1fe540677d05c162ea57506108944c6d893e640705c75a0157c72b7616f3e6a54693781d96b3bf1fe508a019d1649e18ec59c8897384b64e829582f7436a8e98ccec7022772e400", "984fef039e21996cb6098fe41e6d80edb975381d4c4fd577e5f7c1d1d4c58e44a6ec69b319bd723f904e70a5e6c84dbc210895712c48ee6bceaea17fc9547586a9cb481377dc30dd2d320d5ef9b4c64ea203e044d3d06a12c88ecfa63a6d00e82fcbab7408bc6c831e21bc5ee387f40ae53ec11a629aeeab92f1ba24d63d3dd1c50921476c596e353e2df107759f3e0544e72c8d26bb76e66a9b99e1a84d3f5de6789d9a2985f0cc01cc064369d39bac2cfb2ba0cc568172bec693596d00", "3bc8f0092bf478e17c02a36f4dfd88d3fadfda170c7e6bcfaac5f2f8d029d480a6b52146a44b8c916eac337ec8e638484ad792c281dfd0f3a406b72f6bb496d11fd4391a8b5961be3ee485298e0388c37c0dc89a03b0e9d0380589e83c7e0047f4391025fbb3a74ee869e53f09ad2e478ea21bc7a1e4fc5940be93f82ffa3ddf4325d5ea231df7a821df8e9d5be112d64e9b4aec6d5482583df2a672183c2c3ca06ba9596bd0494c684f069e4284eb39add977f68de554dc94e0aff68300", "c9c227188fef26f0381d4aeff67681a464cc3fbcf56d63cf6b2c442baed9f9226da81a2cedeea630a9c7196f45c0ce86018070c630c8e2b5361ce936ec7bc5b12bc6fee85eef75a8a04726cae0f680126bbb712783eedee177058a22cbfa00a3891690657602d3413afa3d050a74c1d0b57dc2aeef3c8a84df27a5e0bd342b390e08e0b088ce749b8fc447e9d592b0e25fa45fe93692da3b97a27d2925f4c2f584acb29d74fdbcf39172e8fc38449d3bd8ee26bca67c434c73a3112b9801", "d4e66a8967081983aa6f740b6da5c2792dae25413524dfcf9f9dcff79ee1ba7224e790142a04910255a45dba35474ee5926b1697d9ef83335569c8f7b1309e44a19c5df7ed31567b846f5909bbf550cafd153f98914f8a8f661754c5c7c20058e730883b6b48b194cd32ec342bb63b7cebc9d7859022abefe7afa9fe4d17b72be36fb429c03cad06cbc621a608369e925efa27b7bee61c5896df5ef64ce33fbe98500b716aff25e51a21d23895a8b9c506afa11f61f7cc66ee91fab15700", "b1cf3f4cbcd53cc60f403147b844910b9126ce96486e83aa6e792bb662a0058eaa22a37446ead4641dbeb75b7f3ae74759d01602d40b38e61a5bf1373c3af9aed76624196213c22ccb6782b32c4646d537e48e4819f9b53050ebfcd5256500807a2c87edd47a91b70996c60a7d7be822ea467d491687181d20aa7dee749486dd5409660136024306aa792288450e5e13286ab6387796d805fc583707436998fd4dd43ae43104aab707482c3c72d51e65ff706adfe3f4d3df6fe964da8e00"];

fn deserialize_generators() -> Vec<G1Projective> {
    let mut generators = Vec::with_capacity(PEDERSEN_GENERATORS.len());
    for generator in PEDERSEN_GENERATORS {
        let decoded_generator = hex::decode(generator).unwrap();
        let generator: G1Projective =
            CanonicalDeserialize::deserialize_unchecked(&decoded_generator[..]).unwrap();
        generators.push(generator);
    }
    generators
}

/// Creates a Merkle tree from the given inputs, as a vector of vectors of booleans, and outputs
/// the root. Each vector of booleans is meant to be one leaf. Each leaf can be of a different
/// size. Number of leaves has to be a power of two.
/// The tree is constructed from left to right. For example, if we are given inputs {0, 1, 2, 3}
/// then the resulting tree will be:
///                      o
///                    /   \
///                   o     o
///                  / \   / \
///                 0  1  2  3
pub fn merkle_tree_construct(inputs: Vec<Vec<bool>>) -> Vec<u8> {
    // Checking that the inputs vector is not empty.
    assert!(!inputs.is_empty());

    // Checking that the number of leaves is a power of two.
    assert!(inputs.len().is_power_of_two());

    // Calculate the required number of Pedersen generators. The formula used for the ceiling
    // division of x/y is (x+y-1)/y.
    let capacity = 752;

    let mut generators_needed = 4; // At least this much is required for the non-leaf nodes.

    for input in &inputs {
        generators_needed = cmp::max(
            generators_needed,
            (input.len() + capacity - 1) / capacity + 1,
        );
    }

    let mut generators = deserialize_generators();
    assert!(
        generators_needed <= generators.len(),
        "Invalid number of pedersen generators"
    );
    generators.truncate(generators_needed);

    // Calculate the Pedersen hashes for the leaves.
    let mut nodes: Vec<G1Projective> = inputs
        .par_iter()
        .map(|bits| pedersen_hash(bits.clone(), generators.clone()))
        .collect();

    // Process each level of nodes.
    while nodes.len() > 1 {
        // Serialize all the child nodes.
        let bits: Vec<bool> = nodes
            .par_iter()
            .map(|node| bytes_to_bits(&serialize_g1_mnt6(node)))
            .flatten()
            .collect();

        // Chunk the bits into the number of parent nodes.
        let mut chunks = Vec::new();

        let num_chunks = nodes.len() / 2;

        for i in 0..num_chunks {
            chunks.push(
                bits[i * bits.len() / num_chunks..(i + 1) * bits.len() / num_chunks].to_vec(),
            );
        }

        // Calculate the parent nodes.
        let mut next_nodes: Vec<G1Projective> = chunks
            .par_iter()
            .map(|bits| pedersen_hash(bits.clone(), generators.clone()))
            .collect();

        // Clear the child nodes and add the parent nodes.
        nodes.clear();
        nodes.append(&mut next_nodes);
    }

    // Serialize the root node.
    let bytes = serialize_g1_mnt6(&nodes[0]);

    Vec::from(bytes.as_ref())
}

/// Verifies a Merkle proof. More specifically, given an input and all of the tree nodes up to
/// the root, it checks if the input is part of the Merkle tree or not. The path is simply the
/// position of the input leaf in little-endian binary. For example, for the given tree:
///                      o
///                    /   \
///                   o     o
///                  / \   / \
///                 0  1  2  3
/// The path for the leaf 2 is simply 01. Another way of thinking about it is that if you go up
/// the tree, each time you are the left node it's a zero and if you are the right node it's an
/// one.
pub fn merkle_tree_verify(
    input: Vec<bool>,
    nodes: Vec<G1Projective>,
    path: Vec<bool>,
    root: Vec<u8>,
) -> bool {
    // Checking that the inputs vector is not empty.
    assert!(!input.is_empty());

    // Checking that the nodes vector is not empty.
    assert!(!nodes.is_empty());

    // Checking that there is one node for each path bit.
    assert_eq!(nodes.len(), path.len());

    // Calculate the required number of Pedersen generators. The formula used for the ceiling
    // division of x/y is (x+y-1)/y.
    let capacity = 752;

    let generators_needed = cmp::max(4, (input.len() + capacity - 1) / capacity + 1);

    let generators = pedersen_generators(generators_needed);

    // Calculate the Pedersen hashes for the input.
    let mut result = pedersen_hash(input, generators.clone());

    // Calculate the root of the tree using the branch values.
    let mut left_node;

    let mut right_node;

    for i in 0..nodes.len() {
        // Decide which node is the left or the right one based on the path.
        if path[i] {
            left_node = nodes[i];
            right_node = result;
        } else {
            left_node = result;
            right_node = nodes[i];
        }

        // Serialize the left and right nodes.
        let mut bytes = Vec::new();

        bytes.extend_from_slice(serialize_g1_mnt6(&left_node).as_ref());

        bytes.extend_from_slice(serialize_g1_mnt6(&right_node).as_ref());

        let bits = bytes_to_bits(&bytes);

        // Calculate the parent node and update result.
        result = pedersen_hash(bits, generators.clone());
    }

    // Serialize the root node.
    let bytes = serialize_g1_mnt6(&result);

    let reference = Vec::from(bytes.as_ref());

    // Check if the calculated root is equal to the given root.
    root == reference
}

/// Creates a Merkle proof given all the leaf nodes and the path to the node for which we want the
/// proof. Basically, it just constructs the whole tree while storing all the intermediate nodes
/// needed for the Merkle proof. It does not output either the root or the input leaf node since
/// these are assumed to be already known by the verifier.
/// The path is simply the position of the input leaf in little-endian binary. For example, for the
/// given tree:
///                      o
///                    /   \
///                   o     o
///                  / \   / \
///                 0  1  2  3
/// The path for the leaf 2 is simply 01. Another way of thinking about it is that if you go up
/// the tree, each time you are the left node it's a zero and if you are the right node it's an
/// one.
pub fn merkle_tree_prove(inputs: Vec<Vec<bool>>, path: Vec<bool>) -> Vec<G1Projective> {
    // Checking that the inputs vector is not empty.
    assert!(!inputs.is_empty());

    // Checking that the number of leaves is a power of two.
    assert_eq!((inputs.len() & (inputs.len() - 1)), 0);

    // Check that the path is of the right size.
    assert_eq!(2_u32.pow(path.len() as u32), inputs.len() as u32);

    // Calculate the required number of Pedersen generators. The formula used for the ceiling
    // division of x/y is (x+y-1)/y.
    let capacity = 752;

    let mut generators_needed = 4; // At least this much is required for the non-leaf nodes.

    for input in &inputs {
        generators_needed = cmp::max(
            generators_needed,
            (input.len() + capacity - 1) / capacity + 1,
        );
    }

    let generators = pedersen_generators(generators_needed);

    // Calculate the Pedersen hashes for the leaves.
    let mut nodes = Vec::new();

    for input in inputs {
        let hash = pedersen_hash(input, generators.clone());
        nodes.push(hash);
    }

    // Calculate the rest of the tree
    let mut next_nodes = Vec::new();

    let mut proof = Vec::new();

    let mut i = 0;

    while nodes.len() > 1 {
        // Calculate the position of the node needed for the proof.
        let proof_position = byte_from_le_bits(&path[i..]) as usize;

        // Process each level of nodes.
        for j in 0..nodes.len() / 2 {
            let mut bytes = Vec::new();

            // Store the proof node, if applicable.
            if proof_position == 2 * j {
                proof.push(nodes[2 * j + 1]);
            }

            if proof_position == 2 * j + 1 {
                proof.push(nodes[2 * j]);
            }

            // Serialize the left node.
            bytes.extend_from_slice(serialize_g1_mnt6(&nodes[2 * j]).as_ref());

            // Serialize the right node.
            bytes.extend_from_slice(serialize_g1_mnt6(&nodes[2 * j + 1]).as_ref());

            // Calculate the parent node.
            let bits = bytes_to_bits(&bytes);
            let parent_node = pedersen_hash(bits, generators.clone());

            next_nodes.push(parent_node);
        }
        nodes.clear();

        nodes.append(&mut next_nodes);

        i += 1;
    }

    proof
}
