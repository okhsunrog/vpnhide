// Trusted CI only. Uses the existing Android keystore; never exports a private
// key or generates a replacement identity. Java17 source-file launcher.
import java.math.BigInteger;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.security.AlgorithmParameters;
import java.security.KeyStore;
import java.security.PrivateKey;
import java.security.PublicKey;
import java.security.Signature;
import java.security.interfaces.ECPublicKey;
import java.security.interfaces.RSAPublicKey;
import java.security.spec.ECGenParameterSpec;
import java.security.spec.ECParameterSpec;
import java.util.Arrays;
import java.util.HexFormat;

class CoreDescriptorSigner {
    private static final byte[] DOMAIN =
        "VPNHIDE-CORE-REPLACEMENT-v1\n".getBytes(StandardCharsets.UTF_8);

    private static String hex(byte[] bytes) {
        return HexFormat.of().formatHex(bytes);
    }
    private static byte[] unsigned(BigInteger value) {
        byte[] bytes = value.toByteArray();
        return bytes.length > 1 && bytes[0] == 0 ? Arrays.copyOfRange(bytes, 1, bytes.length) : bytes;
    }
    private static byte[] coordinate(BigInteger value, int size) {
        byte[] raw = unsigned(value);
        if (raw.length > size) throw new IllegalArgumentException();
        byte[] result = new byte[size];
        System.arraycopy(raw, 0, result, size - raw.length, raw.length);
        return result;
    }
    private static boolean curve(ECPublicKey key, String name) throws Exception {
        AlgorithmParameters parameters = AlgorithmParameters.getInstance("EC");
        parameters.init(new ECGenParameterSpec(name));
        ECParameterSpec expected = parameters.getParameterSpec(ECParameterSpec.class);
        ECParameterSpec actual = key.getParams();
        return expected.getCurve().equals(actual.getCurve())
            && expected.getGenerator().equals(actual.getGenerator())
            && expected.getOrder().equals(actual.getOrder())
            && expected.getCofactor() == actual.getCofactor();
    }
    private record Identity(String json, String algorithm) {}
    private static Identity identity(PublicKey key) throws Exception {
        if (key instanceof RSAPublicKey rsa) {
            int bits = rsa.getModulus().bitLength();
            if (bits < 2048 || bits > 8192 || rsa.getPublicExponent().bitLength() > 64)
                throw new IllegalArgumentException();
            return new Identity("{\"algorithm\":\"rsa_sha256\",\"modulus\":\""
                + hex(unsigned(rsa.getModulus())) + "\",\"exponent\":\""
                + hex(unsigned(rsa.getPublicExponent())) + "\"}", "SHA256withRSA");
        }
        if (key instanceof ECPublicKey ec) {
            boolean p256 = curve(ec, "secp256r1");
            if (!p256 && !curve(ec, "secp384r1")) throw new IllegalArgumentException();
            int size = p256 ? 32 : 48;
            String point = "04" + hex(coordinate(ec.getW().getAffineX(), size))
                + hex(coordinate(ec.getW().getAffineY(), size));
            return new Identity("{\"algorithm\":\"" + (p256 ? "p256_sha256" : "p384_sha384")
                + "\",\"point\":\"" + point + "\"}", p256 ? "SHA256withECDSA" : "SHA384withECDSA");
        }
        throw new IllegalArgumentException();
    }
    public static void main(String[] args) {
        try {
            if (args.length != 4 || !(args[0].equals("public") || args[0].equals("sign")))
                throw new IllegalArgumentException();
            String secret = System.getenv("KEYSTORE_PASSWORD");
            String alias = System.getenv("KEY_ALIAS");
            if (secret == null || secret.isEmpty() || alias == null || alias.isEmpty())
                throw new IllegalArgumentException();
            char[] password = secret.toCharArray();
            KeyStore store = KeyStore.getInstance(Path.of(args[1]).toFile(), password);
            if (!store.isKeyEntry(alias)) throw new IllegalArgumentException();
            PublicKey publicKey = store.getCertificate(alias).getPublicKey();
            Identity id = identity(publicKey);
            byte[] output;
            if (args[0].equals("public")) {
                output = ("{\"schema\":1,\"keys\":[" + id.json() + "]}\n")
                    .getBytes(StandardCharsets.UTF_8);
            } else {
                Path descriptor = Path.of(args[2]);
                if (Files.size(descriptor) > 4096 || Files.size(descriptor) == 0)
                    throw new IllegalArgumentException();
                byte[] body = Files.readAllBytes(descriptor);
                Signature signer = Signature.getInstance(id.algorithm());
                signer.initSign((PrivateKey)store.getKey(alias, password));
                signer.update(DOMAIN);
                signer.update(body);
                output = signer.sign();
                Signature verifier = Signature.getInstance(id.algorithm());
                verifier.initVerify(publicKey);
                verifier.update(DOMAIN);
                verifier.update(body);
                if (!verifier.verify(output)) throw new IllegalArgumentException();
            }
            Arrays.fill(password, '\0');
            Files.write(Path.of(args[3]), output);
        } catch (Exception error) {
            // Provider exceptions may contain key aliases, paths or input data.
            System.err.println("Core descriptor signing failed; no fallback identity is generated.");
            System.exit(1);
        }
    }
}
