package server

import (
	"bytes"
	"net/http"
	"net/http/httptest"
	"os"
	"testing"
	"time"

	"github.com/iyazerski/pdf-tools/internal/pdf"
	"github.com/iyazerski/pdf-tools/internal/session"
	"github.com/iyazerski/pdf-tools/internal/uploads"
)

func newTestServer(t *testing.T) *Server {
	t.Helper()
	secret := []byte("secret")
	srv, err := New(Config{
		Credentials:       session.NewCredentials("test", "test", secret),
		Signer:            session.NewSigner(secret, time.Hour),
		CookieSecure:      CookieSecureNever,
		TrustProxyHeaders: true,
		Uploads:           uploads.NewStore(time.Minute, 10),
		PDF:               pdf.NewProcessor(30 * time.Second),
		// The repo root holds the real templates/ and static/ directories.
		Assets: os.DirFS("../.."),
	})
	if err != nil {
		t.Fatal(err)
	}
	return srv
}

func TestRateLimitAppliesOnlyToLogin(t *testing.T) {
	tests := []struct {
		name, method, path string
		attempts           int
		wantLimited        bool
	}{
		// Liveness and readiness probes call /healthz.
		{name: "healthz", method: http.MethodGet, path: "/healthz", attempts: 500, wantLimited: false},
		{name: "login", method: http.MethodPost, path: "/login", attempts: 20, wantLimited: true},
		// Thumbnails load in bursts.
		{name: "api", method: http.MethodGet, path: "/api/page/x/1", attempts: 500, wantLimited: false},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			h := newTestServer(t).Handler()
			limited := false
			for range tt.attempts {
				req := httptest.NewRequestWithContext(t.Context(), tt.method, tt.path, nil)
				req.Header.Set("X-Forwarded-For", "1.2.3.4")
				rec := httptest.NewRecorder()
				h.ServeHTTP(rec, req)
				if rec.Code == http.StatusTooManyRequests {
					limited = true
					break
				}
			}
			if limited != tt.wantLimited {
				t.Errorf("rate limited = %v; want %v", limited, tt.wantLimited)
			}
		})
	}
}

func TestIndexPagesRenderAllPlaceholders(t *testing.T) {
	a := newTestServer(t).assets
	for _, page := range []staticAsset{a.indexAuthed, a.indexAnonymous, a.indexLoginError} {
		if bytes.Contains(page.body, []byte("{{")) {
			t.Error("unrendered placeholder in index page")
		}
	}
	if !bytes.Contains(a.indexAuthed.body, []byte(`data-authed="1"`)) {
		t.Error(`authed page lacks data-authed="1"`)
	}
	if !bytes.Contains(a.indexLoginError.body, []byte(`data-login-error="1"`)) {
		t.Error(`login error page lacks data-login-error="1"`)
	}
}
