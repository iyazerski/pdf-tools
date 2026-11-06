// Package session checks login credentials and issues signed session tokens.
package session

import (
	"crypto/hmac"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json/v2"
	"strings"
	"time"
)

// Credentials holds the login credentials as MACs, so checks run in constant time.
type Credentials struct {
	key         []byte
	usernameTag []byte
	passwordTag []byte
}

// NewCredentials returns Credentials that accept only username and password.
func NewCredentials(username, password string, key []byte) *Credentials {
	return &Credentials{key: key, usernameTag: mac(key, username), passwordTag: mac(key, password)}
}

// Matches reports whether username and password are correct, without leaking through
// timing how much of them matched.
func (c *Credentials) Matches(username, password string) bool {
	usernameOK := hmac.Equal(mac(c.key, username), c.usernameTag)
	passwordOK := hmac.Equal(mac(c.key, password), c.passwordTag)
	return usernameOK && passwordOK
}

// Signer issues and verifies session tokens of the form "v1.<payload>.<signature>".
type Signer struct {
	key []byte
	ttl time.Duration
}

// NewSigner returns a Signer whose tokens expire after ttl.
func NewSigner(key []byte, ttl time.Duration) *Signer {
	return &Signer{key: key, ttl: ttl}
}

type payload struct {
	Username string `json:"u"`
	ExpUnix  int64  `json:"exp_unix"`
}

var b64 = base64.RawURLEncoding

// Issue returns a token for username that is valid from now until the signer's TTL passes.
func (s *Signer) Issue(username string, now time.Time) string {
	body, err := json.Marshal(payload{Username: username, ExpUnix: now.Add(s.ttl).Unix()})
	if err != nil {
		// Marshaling a string and an integer can't fail.
		panic(err)
	}
	encoded := b64.EncodeToString(body)
	return "v1." + encoded + "." + b64.EncodeToString(mac(s.key, encoded))
}

// Verify returns the username in token if its signature is valid and it hasn't expired.
func (s *Signer) Verify(token string, now time.Time) (string, bool) {
	version, rest, ok := strings.Cut(token, ".")
	if !ok || version != "v1" {
		return "", false
	}
	encoded, sigB64, ok := strings.Cut(rest, ".")
	if !ok || strings.Contains(sigB64, ".") {
		return "", false
	}
	sig, err := b64.DecodeString(sigB64)
	if err != nil || !hmac.Equal(sig, mac(s.key, encoded)) {
		return "", false
	}

	body, err := b64.DecodeString(encoded)
	if err != nil {
		return "", false
	}
	var p payload
	if err := json.Unmarshal(body, &p); err != nil || p.ExpUnix <= now.Unix() {
		return "", false
	}
	return p.Username, true
}

func mac(key []byte, value string) []byte {
	h := hmac.New(sha256.New, key)
	h.Write([]byte(value))
	return h.Sum(nil)
}
