from pprintpp import pprint

from binascii import unhexlify

from nimiqrpc.albatross import AlbatrossApi


nimiq = AlbatrossApi("http://localhost:8648")

print("Validator Key:")
pprint(nimiq.validator_key())

address_privkey = unhexlify("b8de5f15d0da1b67769b6b5ef00e1755a82160943b4c18c0e70dad02f4432aac")
address = "NQ97 DEL2 ELYS RXHX 6X8K JKUF 0UR5 A1D2 J1R6"
#address_raw = unhexlify("6ba82753facfa3e3791394f8f07325505a290726")

print("Import Wallet: {}".format(address))
print(nimiq.import_raw_key(address_privkey))

print("Unlocking account: {}".format(address))
pprint(nimiq.unlock_account(address))

print("Staking")
ret = nimiq.stake(address, address, 100000000)
pprint(ret)
