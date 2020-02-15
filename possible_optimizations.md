Performance optimizations:

Security optimizations:
1) During the public key addition we are first adding the generator and then subtracting it at the end. Either find a different workaround or hard-code a different generator with an unknown secret key (using public randomness and hash-to-curve).
