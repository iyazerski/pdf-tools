package session

import (
	"strings"
	"testing"
	"time"
)

func TestCredentialsMatchOnlyExactUsernameAndPassword(t *testing.T) {
	creds := NewCredentials("admin", "secret", []byte("key"))
	tests := []struct {
		username, password string
		want               bool
	}{
		{"admin", "secret", true},
		{"admin", "secret2", false},
		{"admin2", "secret", false},
		{"", "", false},
	}
	for _, tt := range tests {
		if got := creds.Matches(tt.username, tt.password); got != tt.want {
			t.Errorf("Matches(%q, %q) = %v; want %v", tt.username, tt.password, got, tt.want)
		}
	}
}

func TestSignerVerify(t *testing.T) {
	now := time.Unix(1_700_000_000, 0)
	signer := NewSigner([]byte("key"), time.Hour)
	token := signer.Issue("admin", now)

	tests := []struct {
		name   string
		signer *Signer
		token  string
		now    time.Time
		wantOK bool
	}{
		{name: "valid", signer: signer, token: token, now: now, wantOK: true},
		{name: "expired", signer: signer, token: token, now: now.Add(time.Hour), wantOK: false},
		{name: "other key", signer: NewSigner([]byte("other"), time.Hour), token: token, now: now, wantOK: false},
		{name: "tampered", signer: signer, token: strings.Replace(token, "v1.", "v1.x", 1), now: now, wantOK: false},
		{name: "extra part", signer: signer, token: token + ".x", now: now, wantOK: false},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			username, ok := tt.signer.Verify(tt.token, tt.now)
			if ok != tt.wantOK || (ok && username != "admin") {
				t.Errorf("Verify = %q, %v; want admin, %v", username, ok, tt.wantOK)
			}
		})
	}
}
