// tweetnacl needs a secure random source; Hermes has no crypto.getRandomValues.
import nacl from "tweetnacl";
import { getRandomValues } from "expo-crypto";

nacl.setPRNG((out, n) => {
  const bytes = getRandomValues(new Uint8Array(n));
  for (let i = 0; i < n; i++) out[i] = bytes[i];
});
