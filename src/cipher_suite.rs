use crate::header::Header;
use crate::header::SignedData;
use oqs::sig::{self, Algorithm, PublicKey, SecretKey, Sig};
use pkcs1::DecodeRsaPublicKey;
use rsa::pkcs1::EncodeRsaPublicKey;
use rsa::pkcs1v15::SigningKey;
use rsa::pkcs1v15::VerifyingKey;
use rsa::sha2::Sha256 as rsa_sha2_Sha256;
use rsa::signature::{Keypair, RandomizedSigner, SignatureEncoding, Verifier};
use rsa::RsaPrivateKey;
use rsa::RsaPublicKey;
use serde;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256, Sha512};
use std::{
    fs::{File, OpenOptions},
    io::{self, Read, Write},
};

// An interface for a suite of cryptographic algorithms
// Contains an id, a signature scheme, and a hash function
pub trait CipherSuite {
    fn hash(&self, buffer: &[u8]) -> Vec<u8>;
    fn sign(&self, input: &str, output: &str) -> io::Result<()>;
    fn verify(&self, header: &str) -> io::Result<()>;
    fn get_name(&self) -> &String;
    fn get_pk_bytes(&self) -> Vec<u8>;
    fn get_cs_id(&self) -> usize;
    fn to_enum(&self) -> CS;
    fn print_pk(&self);
    fn peer_verify(&self, signed_data: SignedData, pk: Vec<u8>, cs_id: usize) -> io::Result<()>;
}

// A wrapper for a trait object that allows for stream deserialization
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum CS {
    CS1(Dilithium2Sha256),
    CS2(Dilithium2Sha512),
    CS3(Falcon512Sha256),
    CS4(Falcon512Sha512),
    CS5(RsaSha256),
}

impl CS {
    // Converts a CS enum to a Box pointer of the data type the enum held
    pub fn to_box(self) -> Box<dyn CipherSuite> {
        match self {
            CS::CS1(cs) => Box::new(cs),
            CS::CS2(cs) => Box::new(cs),
            CS::CS3(cs) => Box::new(cs),
            CS::CS4(cs) => Box::new(cs),
            CS::CS5(cs) => Box::new(cs),
        }
    }
}

// Creates a new ciphersuite object based on cs_id
pub fn create_ciphersuite(name: String, cs_id: usize) -> Result<CS, io::Error> {
    let lower_name = name.to_lowercase();

    match cs_id {
        1 => Ok(CS::CS1(Dilithium2Sha256::new(lower_name.clone(), cs_id))),
        2 => Ok(CS::CS2(Dilithium2Sha512::new(lower_name.clone(), cs_id))),
        3 => Ok(CS::CS3(Falcon512Sha256::new(lower_name.clone(), cs_id))),
        4 => Ok(CS::CS4(Falcon512Sha512::new(lower_name.clone(), cs_id))),
        5 => Ok(CS::CS5(RsaSha256::new(lower_name.clone(), cs_id))),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Unsupported cipher suite id. Enter a value between 1-5",
        )),
    }
}

// Sha256 hash function
fn sha256_hash(buffer: &[u8]) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(buffer);
    hasher.finalize().to_vec()
}

// Sha512 hash function
fn sha512_hash(buffer: &[u8]) -> Vec<u8> {
    let mut hasher = Sha512::new();
    hasher.update(buffer);
    hasher.finalize().to_vec()
}

// Hashes input data based on the specified cipher suite ID.
fn hash_based_on_cs_id(cs_id: usize, data: &[u8]) -> io::Result<Vec<u8>> {
    match cs_id {
        1 | 3 | 5 => Ok(sha256_hash(data)),
        2 | 4 => Ok(sha512_hash(data)),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Unsupported cipher suite id",
        )),
    }
}

// Helper function for verify functions
pub fn read_and_deserialize(path: &str) -> io::Result<SignedData> {
    let mut file = File::open(path)?;
    let mut contents = Vec::new();
    file.read_to_end(&mut contents)?;
    serde_cbor::from_slice(&contents)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("Serialization failed: {}", e)))
}

pub fn quantum_sign(
    cs_id: usize,
    contents: Vec<u8>,
    _file_hash: Vec<u8>,
    length: usize,
    output: &str,
    sig_algo: Sig,
    pk_bytes: Vec<u8>,
    sk: &SecretKey,
) -> io::Result<()> {
    // Determine which hash function to use based on cs_id
    let file_hash = hash_based_on_cs_id(cs_id, &contents);

    // Create header
    let header = Header::new(cs_id, length, file_hash?, pk_bytes);

    // Serialize the header
    let header_bytes = serde_cbor::to_vec(&header).map_err(|e| {
        io::Error::new(io::ErrorKind::Other, format!("Serialization failed: {}", e))
    })?;

    // Hash the header bytes
    let hashed_header = hash_based_on_cs_id(cs_id, &header_bytes);

    // Sign the hash
    let signature = sig_algo
        .sign(&hashed_header?, sk)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("Signing failed: {}", e)))?;

    let signed_data = SignedData::new(header, signature.clone().into_vec(), contents);

    // Serialize the SignedData
    let signed_data_str = serde_cbor::to_vec(&signed_data).map_err(|e| {
        io::Error::new(io::ErrorKind::Other, format!("Serialization failed: {}", e))
    })?;

    // Write serialized SignedData to output file
    let mut out_file = OpenOptions::new().write(true).create(true).open(output)?;
    Write::write_all(&mut out_file, &signed_data_str)?;

    Ok(())
}

pub fn quantum_verify(input: &str, sig_algo: Sig, pk: Vec<u8>, cs_id: usize) -> io::Result<()> {
    // Create signed data with helper
    let signed_data = read_and_deserialize(input)?;

    // Declare helper function
    let header = signed_data.get_header();

    // Verify message len
    signed_data.verify_message_len();

    // Verify sender, length of message, and hash of message
    header.verify_sender(pk.clone());

    // Verify hash
    header.verify_hash(&hash_based_on_cs_id(cs_id, signed_data.get_contents())?);

    // Verify file type
    header.verify_file_type();

    // Serialize the header part of the SignedData for hashing
    let header_bytes = serde_cbor::to_vec(&signed_data.get_header()).map_err(|e| {
        io::Error::new(io::ErrorKind::Other, format!("Serialization failed: {}", e))
    })?;

    // Re-hash the serialized header
    let hashed_header_result = hash_based_on_cs_id(cs_id, &header_bytes);

    let hashed_header = match hashed_header_result {
        Ok(hashed) => hashed,
        Err(e) => return Err(e),
    };

    // Convert Vec<u8> to SignatureRef for verification
    let signature_ref = sig_algo
        .signature_from_bytes(signed_data.get_signature())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "Failed to create signature reference",
            )
        })?;

    // Need to convert bytes back into public key for verification
    let pk = sig_algo.public_key_from_bytes(&pk).unwrap();

    // Verify the signature using the provided public key and the hash
    sig_algo
        .verify(&hashed_header, signature_ref, pk)
        .map_err(|e| {
            io::Error::new(
                io::ErrorKind::Other,
                format!("OQS error: Verification failed - {}", e),
            )
        })?;

    Ok(())
}

pub fn quantum_peer_verify(
    signed_data: SignedData,
    cs_id: usize,
    pk: Vec<u8>,
    sig_algo: Sig,
) -> io::Result<()> {
    // Declare helper function
    let header = signed_data.get_header();

    // Verify message len
    signed_data.verify_message_len();

    // Verify sender, length of message, and hash of message
    header.verify_sender(pk.clone());

    // Verify hash
    header.verify_hash(&hash_based_on_cs_id(cs_id, signed_data.get_contents())?);

    // Verify file type
    header.verify_file_type();

    // Serialize the header part of the SignedData for hashing
    let header_bytes = serde_cbor::to_vec(&signed_data.get_header()).map_err(|e| {
        io::Error::new(io::ErrorKind::Other, format!("Serialization failed: {}", e))
    })?;

    // Re-hash the serialized header
    let hashed_header_result = hash_based_on_cs_id(cs_id, &header_bytes);

    let hashed_header = match hashed_header_result {
        Ok(hashed) => hashed,
        Err(e) => return Err(e),
    };

    // Convert Vec<u8> to SignatureRef for verification
    let signature_ref = sig_algo
        .signature_from_bytes(signed_data.get_signature())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "Failed to create signature reference",
            )
        })?;

    // Need to convert bytes back into public key for verification
    let pk = sig_algo.public_key_from_bytes(&pk).unwrap();

    // Verify the signature using the provided public key and the hash
    sig_algo
        .verify(&hashed_header, signature_ref, pk)
        .map_err(|e| {
            io::Error::new(
                io::ErrorKind::Other,
                format!("OQS error: Verification failed - {}", e),
            )
        })?;

    Ok(())
}
// A ciphersuite that uses Dilithium2 signature scheme and sha-256 hashing
// CS_ID: 1
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Dilithium2Sha256 {
    name: String,
    cs_id: usize,
    pk: PublicKey,
    sk: SecretKey,
}

impl Dilithium2Sha256 {
    pub fn new(name: String, cs_id: usize) -> Self {
        let sig_algo =
            sig::Sig::new(sig::Algorithm::Dilithium2).expect("Failed to create sig object");
        let (pk, sk) = sig_algo.keypair().expect("Failed to generate keypair");

        Dilithium2Sha256 {
            name,
            cs_id,
            pk,
            sk,
        }
    }

    pub fn get_pk(&self) -> PublicKey {
        self.pk.clone()
    }
}

impl CipherSuite for Dilithium2Sha256 {
    fn hash(&self, buffer: &[u8]) -> Vec<u8> {
        sha256_hash(buffer)
    }

    fn sign(&self, input: &str, output: &str) -> io::Result<()> {
        // Read and hash the input file
        let mut in_file = File::open(input)?;
        let mut contents = Vec::new();
        let length = in_file.read_to_end(&mut contents)?;
        let file_hash: Vec<u8> = self.hash(&contents);

        // Create sig object
        let sig_algo = Sig::new(Algorithm::Dilithium2).expect("Unable to create sig object");

        // Sign file
        quantum_sign(
            self.cs_id,
            contents,
            file_hash,
            length,
            output,
            sig_algo,
            self.get_pk_bytes(),
            &self.sk,
        )
    }

    fn verify(&self, input: &str) -> io::Result<()> {
        let sig_algo = Sig::new(Algorithm::Dilithium2).expect("Failed to create sig object");

        quantum_verify(input, sig_algo, self.get_pk_bytes(), self.cs_id)
    }

    fn get_name(&self) -> &String {
        &self.name
    }

    fn get_pk_bytes(&self) -> Vec<u8> {
        self.get_pk().into_vec()
    }

    fn get_cs_id(&self) -> usize {
        self.cs_id
    }

    fn to_enum(&self) -> CS {
        CS::CS1(self.clone())
    }

    fn print_pk(&self) {
        println!("{:?}", self.get_pk_bytes())
    }

    fn peer_verify(&self, signed_data: SignedData, pk: Vec<u8>, cs_id: usize) -> io::Result<()> {
        let sig_algo = Sig::new(Algorithm::Dilithium2).expect("Failed to create sig object");

        quantum_peer_verify(signed_data, cs_id, pk, sig_algo)
    }
}

// A ciphersuite that uses Dilithium2 signature scheme and sha-512 hashing
// CS_ID 2
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Dilithium2Sha512 {
    name: String,
    cs_id: usize,
    pk: PublicKey,
    sk: SecretKey,
}

impl Dilithium2Sha512 {
    pub fn new(name: String, cs_id: usize) -> Self {
        let sig_algo =
            sig::Sig::new(sig::Algorithm::Dilithium2).expect("Failed to create sig object");
        let (pk, sk) = sig_algo.keypair().expect("Failed to generate keypair");

        Dilithium2Sha512 {
            name,
            cs_id,
            pk,
            sk,
        }
    }

    pub fn get_pk(&self) -> PublicKey {
        self.pk.clone()
    }
}

impl CipherSuite for Dilithium2Sha512 {
    fn hash(&self, buffer: &[u8]) -> Vec<u8> {
        sha512_hash(buffer)
    }

    fn sign(&self, input: &str, output: &str) -> io::Result<()> {
        // Read and hash the input file
        let mut in_file = File::open(input)?;
        let mut contents = Vec::new();
        let length = in_file.read_to_end(&mut contents)?;
        let file_hash: Vec<u8> = self.hash(&contents);

        // Create sig object
        let sig_algo = Sig::new(Algorithm::Dilithium2).expect("Unable to create sig object");

        // Sign file
        quantum_sign(
            self.cs_id,
            contents,
            file_hash,
            length,
            output,
            sig_algo,
            self.get_pk_bytes(),
            &self.sk,
        )
    }

    fn verify(&self, input: &str) -> io::Result<()> {
        let sig_algo = Sig::new(Algorithm::Dilithium2).expect("Failed to create sig object");

        quantum_verify(input, sig_algo, self.get_pk_bytes(), self.cs_id)
    }

    fn get_name(&self) -> &String {
        &self.name
    }

    fn get_pk_bytes(&self) -> Vec<u8> {
        self.get_pk().into_vec()
    }

    fn get_cs_id(&self) -> usize {
        self.cs_id
    }

    fn to_enum(&self) -> CS {
        CS::CS2(self.clone())
    }

    fn print_pk(&self) {
        println!("{:?}", self.get_pk_bytes())
    }

    fn peer_verify(&self, signed_data: SignedData, pk: Vec<u8>, cs_id: usize) -> io::Result<()> {
        let sig_algo = Sig::new(Algorithm::Dilithium2).expect("Failed to create sig object");

        quantum_peer_verify(signed_data, cs_id, pk, sig_algo)
    }
}

// // A ciphersuite that uses Falcon512 signature scheme and sha-256 hashing
// // CS_ID: 3
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Falcon512Sha256 {
    name: String,
    cs_id: usize,
    pk: PublicKey,
    sk: SecretKey,
}

impl Falcon512Sha256 {
    pub fn new(name: String, cs_id: usize) -> Self {
        let sig_algo =
            sig::Sig::new(sig::Algorithm::Falcon512).expect("Failed to create sig object");
        let (pk, sk) = sig_algo.keypair().expect("Failed to generate keypair");

        Falcon512Sha256 {
            name,
            cs_id,
            pk,
            sk,
        }
    }

    pub fn get_pk(&self) -> PublicKey {
        self.pk.clone()
    }
}

impl CipherSuite for Falcon512Sha256 {
    fn hash(&self, buffer: &[u8]) -> Vec<u8> {
        sha256_hash(buffer)
    }

    fn sign(&self, input: &str, output: &str) -> io::Result<()> {
        // Read and hash the input file
        let mut in_file = File::open(input)?;
        let mut contents = Vec::new();
        let length = in_file.read_to_end(&mut contents)?;
        let file_hash: Vec<u8> = self.hash(&contents);

        // Create sig object
        let sig_algo = Sig::new(Algorithm::Falcon512).expect("Unable to create sig object");

        // Sign file
        quantum_sign(
            self.cs_id,
            contents,
            file_hash,
            length,
            output,
            sig_algo,
            self.get_pk_bytes(),
            &self.sk,
        )
    }

    fn verify(&self, input: &str) -> io::Result<()> {
        let sig_algo = Sig::new(Algorithm::Falcon512).expect("Failed to create sig object");

        quantum_verify(input, sig_algo, self.get_pk_bytes(), self.cs_id)
    }

    fn get_name(&self) -> &String {
        &self.name
    }

    fn get_pk_bytes(&self) -> Vec<u8> {
        self.get_pk().into_vec()
    }

    fn get_cs_id(&self) -> usize {
        self.cs_id
    }

    fn to_enum(&self) -> CS {
        CS::CS3(self.clone())
    }

    fn print_pk(&self) {
        println!("{:?}", self.get_pk_bytes())
    }

    fn peer_verify(&self, signed_data: SignedData, pk: Vec<u8>, cs_id: usize) -> io::Result<()> {
        let sig_algo = Sig::new(Algorithm::Falcon512).expect("Failed to create sig object");

        quantum_peer_verify(signed_data, cs_id, pk, sig_algo)
    }
}

// A ciphersuite that uses Falcon512 signature scheme and sha-512 hashing
// CS_ID: 4
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Falcon512Sha512 {
    name: String,
    cs_id: usize,
    pk: PublicKey,
    sk: SecretKey,
}

impl Falcon512Sha512 {
    pub fn new(name: String, cs_id: usize) -> Self {
        let sig_algo =
            sig::Sig::new(sig::Algorithm::Falcon512).expect("Failed to create sig object");
        let (pk, sk) = sig_algo.keypair().expect("Failed to generate keypair");

        Falcon512Sha512 {
            name,
            cs_id,
            pk,
            sk,
        }
    }

    pub fn get_pk(&self) -> PublicKey {
        self.pk.clone()
    }
}

impl CipherSuite for Falcon512Sha512 {
    fn hash(&self, buffer: &[u8]) -> Vec<u8> {
        sha512_hash(buffer)
    }

    fn sign(&self, input: &str, output: &str) -> io::Result<()> {
        // Read and hash the input file
        let mut in_file = File::open(input)?;
        let mut contents = Vec::new();
        let length = in_file.read_to_end(&mut contents)?;
        let file_hash: Vec<u8> = self.hash(&contents);

        // Create sig object
        let sig_algo = Sig::new(Algorithm::Falcon512).expect("Unable to create sig object");

        // Sign file
        quantum_sign(
            self.cs_id,
            contents,
            file_hash,
            length,
            output,
            sig_algo,
            self.get_pk_bytes(),
            &self.sk,
        )
    }

    fn verify(&self, input: &str) -> io::Result<()> {
        let sig_algo = Sig::new(Algorithm::Falcon512).expect("Failed to create sig object");

        quantum_verify(input, sig_algo, self.get_pk_bytes(), self.cs_id)
    }

    fn get_name(&self) -> &String {
        &self.name
    }

    fn get_pk_bytes(&self) -> Vec<u8> {
        self.get_pk().into_vec()
    }

    fn get_cs_id(&self) -> usize {
        self.cs_id
    }

    fn to_enum(&self) -> CS {
        CS::CS4(self.clone())
    }

    fn print_pk(&self) {
        println!("{:?}", self.get_pk_bytes())
    }

    fn peer_verify(&self, signed_data: SignedData, pk: Vec<u8>, cs_id: usize) -> io::Result<()> {
        let sig_algo = Sig::new(Algorithm::Falcon512).expect("Failed to create sig object");

        quantum_peer_verify(signed_data, cs_id, pk, sig_algo)
    }
}

// A ciphersuite that uses RSA PKCS signature scheme and sha-256 hashing
// CS_ID: 5
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RsaSha256 {
    name: String,
    cs_id: usize,
    sk: RsaPrivateKey,
    pk: RsaPublicKey,
}

impl RsaSha256 {
    pub fn new(name: String, cs_id: usize) -> Self {
        let mut rng = rand::thread_rng();
        let bits = 2048;
        let sk = RsaPrivateKey::new(&mut rng, bits).expect("failed to generate a key");
        let pk = RsaPublicKey::from(&sk);

        RsaSha256 {
            name,
            cs_id,
            sk,
            pk,
        }
    }

    pub fn get_pk(&self) -> RsaPublicKey {
        self.pk.clone()
    }
}

impl CipherSuite for RsaSha256 {
    fn hash(&self, buffer: &[u8]) -> Vec<u8> {
        sha256_hash(buffer)
    }

    fn sign(&self, input: &str, output: &str) -> io::Result<()> {
        // Read and hash the input file's contents
        let mut in_file = File::open(input)?;
        let mut contents = Vec::new();
        in_file.read_to_end(&mut contents)?;

        // Serialize header to bytes
        let header = Header::new(
            self.cs_id,
            contents.len(),
            self.hash(&contents),
            self.pk.to_pkcs1_der().unwrap().as_ref().to_vec(),
        );

        let serialized_header = serde_cbor::to_vec(&header).map_err(|e| {
            io::Error::new(io::ErrorKind::Other, format!("Serialization failed: {}", e))
        })?;

        // Hash the serialized header
        let hashed_header = self.hash(&serialized_header);

        // Sign hashed data
        let mut rng = rand::thread_rng();
        let signing_key = SigningKey::<rsa_sha2_Sha256>::new(self.sk.clone());
        let signature = signing_key.sign_with_rng(&mut rng, &hashed_header);
        let signature = signature.to_vec();

        let signed_data = SignedData::new(header, signature, contents);

        // Serialize the SignedData
        let signed_data_str = serde_cbor::to_vec(&signed_data).map_err(|e| {
            io::Error::new(io::ErrorKind::Other, format!("Serialization failed: {}", e))
        })?;

        // Write serialized SignedData to output file
        let mut out_file = OpenOptions::new().write(true).create(true).open(output)?;
        out_file.write_all(&signed_data_str)?;

        Ok(())
    }

    fn verify(&self, input: &str) -> io::Result<()> {
        // Create signed data with helper
        let signed_data = read_and_deserialize(input)?;

        // Verify message len
        signed_data.verify_message_len();

        // Verify file type
        signed_data.get_header().verify_file_type();

        // Serialize the header to bytes
        let header_bytes = serde_cbor::to_vec(&signed_data.get_header()).map_err(|e| {
            io::Error::new(io::ErrorKind::Other, format!("Serialization failed: {}", e))
        })?;

        // Re-hash contents for verification
        let contents_hash = signed_data.get_contents();
        let hash = self.hash(contents_hash);
        signed_data.get_header().verify_hash(&hash);

        // Verify sender
        signed_data
            .get_header()
            .verify_sender(signed_data.get_header().get_pk_bytes().to_vec());

        // Re-hash the serialized header
        let hashed_header = self.hash(&header_bytes);

        // Convert the stored signature into a format suitable for verification
        let signature = rsa::pkcs1v15::Signature::try_from(signed_data.get_signature().as_slice())
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e.to_string()))?;

        // Recreate the signing key
        let signing_key = SigningKey::<rsa_sha2_Sha256>::new(self.sk.clone());

        // Create the verifying key
        let verifying_key = signing_key.verifying_key();

        // Verify the signature
        verifying_key
            .verify(&hashed_header, &signature)
            .map_err(|e| {
                io::Error::new(io::ErrorKind::Other, format!("Verification failed: {}", e))
            })
    }

    fn get_name(&self) -> &String {
        &self.name
    }

    fn get_pk_bytes(&self) -> Vec<u8> {
        self.get_pk()
            .to_pkcs1_der()
            .expect("Failed to serialize public key")
            .to_vec()
    }

    fn get_cs_id(&self) -> usize {
        self.cs_id
    }

    fn to_enum(&self) -> CS {
        CS::CS5(self.clone())
    }

    fn print_pk(&self) {
        println!("{:?}", self.get_pk_bytes())
    }

    fn peer_verify(&self, signed_data: SignedData, pk: Vec<u8>, cs_id: usize) -> io::Result<()> {
        // Check if the cs_id matches
        if signed_data.get_header().get_cs_id() != cs_id {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "CS ID does not match",
            ));
        }

        // Verify message len
        signed_data.verify_message_len();

        // Verify sender, length of message, and hash of message
        signed_data.get_header().verify_sender(pk.clone());

        // Verify Hash
        signed_data
            .get_header()
            .verify_hash(&hash_based_on_cs_id(cs_id, signed_data.get_contents())?);

        // Serialize the header part of the SignedData for hashing
        let header_bytes = serde_cbor::to_vec(&signed_data.get_header()).map_err(|e| {
            io::Error::new(io::ErrorKind::Other, format!("Serialization failed: {}", e))
        })?;

        // Re-hash the serialized header
        let hashed_header_result = hash_based_on_cs_id(cs_id, &header_bytes);

        // Do a quick check
        let hashed_header = match hashed_header_result {
            Ok(hashed) => hashed,
            Err(e) => return Err(e),
        };

        // Load the public key
        let public_key = RsaPublicKey::from_pkcs1_der(&pk).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("Failed to parse public key: {}", e),
            )
        })?;

        // Create a verifier with the provided public key
        let verifying_key: VerifyingKey<rsa_sha2_Sha256> = VerifyingKey::new(public_key);

        // Create signature
        let signature = rsa::pkcs1v15::Signature::try_from(signed_data.get_signature().as_slice())
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e.to_string()))?;

        // Perform the verification of the signature with the hash
        verifying_key
            .verify(&hashed_header, &signature)
            .expect("Verification failed at verifying key\n");

        Ok(())
    }
}
