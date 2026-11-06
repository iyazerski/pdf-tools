package server

import (
	"bytes"
	"encoding/json/v2"
	"fmt"
	"mime/multipart"
	"net/http"
	"net/http/httptest"
	"net/textproto"
	"net/url"
	"os"
	"os/exec"
	"path/filepath"
	"slices"
	"strings"
	"testing"
	"time"

	"github.com/iyazerski/pdf-tools/internal/pdf"
	"github.com/iyazerski/pdf-tools/internal/pdftest"
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
		PDF:               pdf.NewProcessor(30*time.Second, 2),
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

func TestAPIRequiresSession(t *testing.T) {
	h := newTestServer(t).Handler()
	for _, route := range []struct{ method, path string }{
		{http.MethodPost, "/api/npages"},
		{http.MethodPost, "/api/merge"},
		{http.MethodGet, "/api/page/x/1"},
		{http.MethodDelete, "/api/upload/x"},
	} {
		req := httptest.NewRequestWithContext(t.Context(), route.method, route.path, nil)
		rec := httptest.NewRecorder()
		h.ServeHTTP(rec, req)
		if rec.Code != http.StatusUnauthorized {
			t.Errorf("%s %s status = %d; want %d", route.method, route.path, rec.Code, http.StatusUnauthorized)
		}
	}
}

// login logs in to h and returns the session cookie.
func login(t *testing.T, h http.Handler) *http.Cookie {
	t.Helper()
	form := url.Values{"username": {"test"}, "password": {"test"}}
	req := httptest.NewRequestWithContext(t.Context(), http.MethodPost, "/login", strings.NewReader(form.Encode()))
	req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	req.RemoteAddr = "192.0.2.1:1234"
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	for _, c := range rec.Result().Cookies() {
		if c.Name == sessionCookieName {
			return c
		}
	}
	t.Fatalf("login set no session cookie (status %d)", rec.Code)
	return nil
}

func TestUploadPreviewAndMerge(t *testing.T) {
	// Upload and merge directories go to the temporary directory, so clean them up too.
	t.Setenv("TMPDIR", t.TempDir())
	h := newTestServer(t).Handler()
	cookie := login(t, h)
	do := func(req *http.Request) *httptest.ResponseRecorder {
		t.Helper()
		req.AddCookie(cookie)
		rec := httptest.NewRecorder()
		h.ServeHTTP(rec, req)
		if rec.Code != http.StatusOK {
			t.Fatalf("%s %s status = %d: %s", req.Method, req.URL, rec.Code, rec.Body)
		}
		return rec
	}

	// More pages than one thumbnail batch, so the test covers a later batch too.
	widths := make([]int, thumbBatchPages+5)
	for i := range widths {
		widths[i] = 101 + i
	}
	src := filepath.Join(t.TempDir(), "in.pdf")
	pdftest.Write(t, src, widths...)
	var body bytes.Buffer
	mw := multipart.NewWriter(&body)
	header := textproto.MIMEHeader{}
	header.Set("Content-Disposition", `form-data; name="file"; filename="in.pdf"`)
	header.Set("Content-Type", "application/pdf")
	part, err := mw.CreatePart(header)
	if err != nil {
		t.Fatal(err)
	}
	data, err := os.ReadFile(src)
	if err != nil {
		t.Fatal(err)
	}
	_, _ = part.Write(data)
	if err := mw.Close(); err != nil {
		t.Fatal(err)
	}
	req := httptest.NewRequestWithContext(t.Context(), http.MethodPost, "/api/npages", &body)
	req.Header.Set("Content-Type", mw.FormDataContentType())
	var uploaded npagesResponse
	if err := json.Unmarshal(do(req).Body.Bytes(), &uploaded); err != nil {
		t.Fatal(err)
	}
	if uploaded.Pages != len(widths) {
		t.Fatalf("pages = %d; want %d", uploaded.Pages, len(widths))
	}

	t.Run("previews", func(t *testing.T) {
		if _, err := exec.LookPath("gs"); err != nil {
			t.Skip("previews need Ghostscript (gs)")
		}
		for _, query := range []string{"1?kind=thumb", fmt.Sprintf("%d?kind=thumb", len(widths)), "2?kind=full"} {
			req := httptest.NewRequestWithContext(t.Context(), http.MethodGet, "/api/page/"+uploaded.UploadID+"/"+query, nil)
			if rec := do(req); !bytes.HasPrefix(rec.Body.Bytes(), []byte("\xff\xd8")) {
				t.Errorf("page %s is not a JPEG", query)
			}
		}
	})

	layout := fmt.Sprintf(`{"quality": 100, "layout": [{"upload": %q, "page": 3}, {"upload": %[1]q, "page": 1}]}`, uploaded.UploadID)
	req = httptest.NewRequestWithContext(t.Context(), http.MethodPost, "/api/merge", strings.NewReader(layout))
	req.Header.Set("Content-Type", "application/json")
	merged := filepath.Join(t.TempDir(), "merged.pdf")
	if err := os.WriteFile(merged, do(req).Body.Bytes(), 0o600); err != nil {
		t.Fatal(err)
	}
	if got := pdftest.PageWidths(t, merged); !slices.Equal(got, []int{103, 101}) {
		t.Errorf("merged page widths = %v; want [103 101]", got)
	}
}

func TestCookieSecureModeUnmarshalText(t *testing.T) {
	for text, want := range map[string]CookieSecureMode{"auto": CookieSecureAuto, "true": CookieSecureAlways, "0": CookieSecureNever} {
		var got CookieSecureMode
		if err := got.UnmarshalText([]byte(text)); err != nil || got != want {
			t.Errorf("UnmarshalText(%q) = %v, %v; want %v, nil", text, got, err, want)
		}
	}
	var m CookieSecureMode
	if err := m.UnmarshalText([]byte("sometimes")); err == nil {
		t.Error(`UnmarshalText("sometimes") succeeded; want an error`)
	}
}

func TestStaticAssetRevalidatesWithETag(t *testing.T) {
	h := newTestServer(t).Handler()
	req := httptest.NewRequestWithContext(t.Context(), http.MethodGet, "/static/app.js", nil)
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	etag := rec.Header().Get("ETag")
	if rec.Code != http.StatusOK || etag == "" {
		t.Fatalf("GET status = %d, ETag = %q; want 200 and an ETag", rec.Code, etag)
	}

	req = httptest.NewRequestWithContext(t.Context(), http.MethodGet, "/static/app.js", nil)
	req.Header.Set("If-None-Match", etag)
	rec = httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	if rec.Code != http.StatusNotModified {
		t.Errorf("revalidation status = %d; want %d", rec.Code, http.StatusNotModified)
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
