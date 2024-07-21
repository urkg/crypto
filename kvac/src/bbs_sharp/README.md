BBS# as described [here](https://github.com/user-attachments/files/15905230/BBS_Sharp_Short_TR.pdf)

This assumes that the messages/attributes have already been prepared before signing, i.e. attributes are hashed
with public salts, etc and whats called `H_i` in the paper is already created.

Assumes that a Schnorr Signature will be generated by the user's secure hardware.

Implements both the offline and half-offline (HOL) mode.  
In the former, the verifier is either the signer (has the secret key) or can ask the signer to verify the proof without revealing any user-specific info.  
In the latter, the user needs to communicate with the signer before creating a proof and get "some helper data"
to create a proof which the verifier can check without needing the secret key or interacting with the issuer.
For efficiency and avoiding correlation (when signer and verifier collude), the user gets a batch of
"helper data" to let him create several proofs.