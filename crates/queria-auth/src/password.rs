use argon2::password_hash::{
    PasswordHash, PasswordHasher as _, PasswordVerifier as _, SaltString, rand_core::OsRng,
};
use argon2::{Algorithm, Argon2, Params, Version};
use queria_core::QueriaError;
use queria_core::QueriaResult;

#[derive(Clone, Debug, Default)]
pub struct PasswordHasher;

impl PasswordHasher {
    pub fn hash_password(&self, password: &str) -> QueriaResult<String> {
        if password.len() < 12 {
            return Err(QueriaError::Validation(
                "password must be at least 12 characters".to_owned(),
            ));
        }

        let salt = SaltString::generate(&mut OsRng);
        let argon2 = argon2id();
        argon2
            .hash_password(password.as_bytes(), &salt)
            .map(|hash| hash.to_string())
            .map_err(|error| {
                QueriaError::Infrastructure(format!("password hashing failed: {error}"))
            })
    }

    pub fn verify_password(&self, password: &str, encoded_hash: &str) -> QueriaResult<bool> {
        let parsed_hash = PasswordHash::new(encoded_hash).map_err(|error| {
            QueriaError::Infrastructure(format!("invalid password hash: {error}"))
        })?;

        match argon2id().verify_password(password.as_bytes(), &parsed_hash) {
            Ok(()) => Ok(true),
            Err(argon2::password_hash::Error::Password) => Ok(false),
            Err(error) => Err(QueriaError::Infrastructure(format!(
                "password verification failed: {error}"
            ))),
        }
    }
}

fn argon2id() -> Argon2<'static> {
    Argon2::new(Algorithm::Argon2id, Version::V0x13, Params::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_hash_verifies_original_but_not_wrong_password() {
        let hasher = PasswordHasher;

        let hash = hasher
            .hash_password("correct horse battery staple")
            .expect("password should hash");

        assert_ne!(hash, "correct horse battery staple");
        assert!(
            hasher
                .verify_password("correct horse battery staple", &hash)
                .expect("verification should complete")
        );
        assert!(
            !hasher
                .verify_password("wrong password", &hash)
                .expect("verification should complete")
        );
    }
}
