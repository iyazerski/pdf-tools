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
		WorkDir:           t.TempDir(),
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

	// Clients upload the file again when the server reports the upload as missing.
	req = httptest.NewRequestWithContext(t.Context(), http.MethodDelete, "/api/upload/"+uploaded.UploadID, nil)
	req.AddCookie(cookie)
	h.ServeHTTP(httptest.NewRecorder(), req)
	for _, req := range []*http.Request{
		httptest.NewRequestWithContext(t.Context(), http.MethodGet, "/api/page/"+uploaded.UploadID+"/1", nil),
		httptest.NewRequestWithContext(t.Context(), http.MethodPost, "/api/merge", strings.NewReader(layout)),
	} {
		req.Header.Set("Content-Type", "application/json")
		req.AddCookie(cookie)
		rec := httptest.NewRecorder()
		h.ServeHTTP(rec, req)
		if rec.Code != http.StatusNotFound {
			t.Errorf("%s %s after delete status = %d; want %d", req.Method, req.URL, rec.Code, http.StatusNotFound)
		}
		if req.Method == http.MethodPost && rec.Header().Get(missingUploadHeader) != uploaded.UploadID {
			t.Errorf("merge after delete %s = %q; want %q", missingUploadHeader, rec.Header().Get(missingUploadHeader), uploaded.UploadID)
		}
	}
}

func TestIndexShowsLoginState(t *testing.T) {
	h := newTestServer(t).Handler()
	cookie := login(t, h)
	tests := []struct {
		name, path string
		cookie     *http.Cookie
		want       string
	}{
		{name: "anonymous", path: "/", want: `data-authed="0" data-login-error="0"`},
		{name: "login error", path: "/?login_error=1", want: `data-authed="0" data-login-error="1"`},
		{name: "logged in", path: "/", cookie: cookie, want: `data-authed="1" data-login-error="0"`},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			req := httptest.NewRequestWithContext(t.Context(), http.MethodGet, tt.path, nil)
			if tt.cookie != nil {
				req.AddCookie(tt.cookie)
			}
			rec := httptest.NewRecorder()
			h.ServeHTTP(rec, req)
			if rec.Code != http.StatusOK || !strings.Contains(rec.Body.String(), tt.want) {
				t.Errorf("GET %s status = %d, body contains %s = false; want 200 and true", tt.path, rec.Code, tt.want)
			}
		})
	}
}
