// Package server serves the PDF tools web app and its API.
package server

import (
	"errors"
	"fmt"
	"io"
	"io/fs"
	"log/slog"
	"net/http"
	"strconv"
	"strings"
	"time"

	"github.com/iyazerski/pdf-tools/internal/pdf"
	"github.com/iyazerski/pdf-tools/internal/session"
	"github.com/iyazerski/pdf-tools/internal/uploads"
)

const (
	maxPDFs      = 10
	maxFileBytes = 30 << 20
	// Bounds the work and temporary files of one merge.
	maxLayoutPages = 5000

	// Request body limits. An upload adds room for the multipart framing, and a merge
	// request fits maxLayoutPages page references. Everything else is a small form or has
	// no body.
	maxUploadBodyBytes = maxFileBytes + 1<<20
	maxMergeBodyBytes  = 1 << 20
	maxSmallBodyBytes  = 4 << 10

	sessionCookieName = "pdf_tools_session"

	loginRateLimitPeriod = 2 * time.Second
	loginRateLimitBurst  = 5
)

// CookieSecureMode controls the Secure attribute of the session cookie.
type CookieSecureMode int

const (
	// CookieSecureAuto sets Secure when a trusted proxy reports an HTTPS request.
	CookieSecureAuto CookieSecureMode = iota
	CookieSecureAlways
	CookieSecureNever
)

// UnmarshalText parses "auto" or a boolean as accepted by strconv.ParseBool.
func (m *CookieSecureMode) UnmarshalText(text []byte) error {
	v := strings.TrimSpace(string(text))
	if strings.EqualFold(v, "auto") {
		*m = CookieSecureAuto
		return nil
	}
	secure, err := strconv.ParseBool(v)
	if err != nil {
		return fmt.Errorf("must be auto, true, or false (got %q)", v)
	}
	*m = CookieSecureNever
	if secure {
		*m = CookieSecureAlways
	}
	return nil
}

// Config holds the dependencies and settings of a Server.
type Config struct {
	Credentials       *session.Credentials
	Signer            *session.Signer
	CookieSecure      CookieSecureMode
	TrustProxyHeaders bool
	Uploads           *uploads.Store
	PDF               *pdf.Processor
	// Assets must contain templates/index.html and the files in static/.
	Assets fs.FS
}

// Server handles HTTP requests for the app.
type Server struct {
	cfg          Config
	assets       *assets
	loginLimiter *ipLimiter
}

// New returns a Server, or an error if the assets are missing or broken.
func New(cfg Config) (*Server, error) {
	a, err := loadAssets(cfg.Assets)
	if err != nil {
		return nil, fmt.Errorf("load assets: %w", err)
	}
	return &Server{
		cfg:          cfg,
		assets:       a,
		loginLimiter: newIPLimiter(loginRateLimitPeriod, loginRateLimitBurst),
	}, nil
}

// Handler returns the app's HTTP handler.
func (s *Server) Handler() http.Handler {
	mux := http.NewServeMux()

	// Probes and static assets skip request logging to keep logs useful.
	mux.HandleFunc("GET /healthz", func(w http.ResponseWriter, _ *http.Request) {
		_, _ = io.WriteString(w, "ok")
	})
	for path, asset := range s.assets.static {
		mux.Handle("GET "+path, asset)
	}
	mux.Handle("GET /favicon.ico", http.RedirectHandler("/favicon.svg", http.StatusPermanentRedirect))

	s.handle(mux, "GET /{$}", maxSmallBodyBytes, s.index)
	// Only login needs throttling (password guessing). Limiting the API would break bursts
	// of thumbnail requests.
	s.handle(mux, "POST /login", maxSmallBodyBytes, s.rateLimited(s.login))
	s.handle(mux, "POST /logout", maxSmallBodyBytes, s.logout)
	s.handle(mux, "POST /api/npages", maxUploadBodyBytes, s.authed(s.npages))
	s.handle(mux, "POST /api/merge", maxMergeBodyBytes, s.authed(s.merge))
	s.handle(mux, "GET /api/page/{upload}/{page}", maxSmallBodyBytes, s.authed(s.pageImage))
	s.handle(mux, "DELETE /api/upload/{upload}", maxSmallBodyBytes, s.authed(s.deleteUpload))
	return mux
}

// handlerFunc is an HTTP handler that reports failures as an error instead of writing them.
type handlerFunc func(w http.ResponseWriter, r *http.Request) error

// handle registers h for pattern, with request logging and a request body of at most
// maxBody bytes.
func (s *Server) handle(mux *http.ServeMux, pattern string, maxBody int64, h handlerFunc) {
	mux.Handle(pattern, logRequests(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		r.Body = http.MaxBytesReader(w, r.Body, maxBody)
		if err := h(w, r); err != nil {
			writeError(w, r, err)
		}
	})))
}

// httpError is an error with a status and a message that are safe to show to clients.
type httpError struct {
	status int
	msg    string
}

func (e *httpError) Error() string { return e.msg }

var errUnauthorized = &httpError{status: http.StatusUnauthorized, msg: "Unauthorized"}

func badRequest(format string, args ...any) error {
	return &httpError{status: http.StatusBadRequest, msg: fmt.Sprintf(format, args...)}
}

func writeError(w http.ResponseWriter, r *http.Request, err error) {
	if httpErr, ok := errors.AsType[*httpError](err); ok {
		http.Error(w, httpErr.msg, httpErr.status)
		return
	}
	if _, ok := errors.AsType[*http.MaxBytesError](err); ok {
		http.Error(w, "Request body is too large", http.StatusRequestEntityTooLarge)
		return
	}
	if errors.Is(err, pdf.ErrBusy) {
		slog.Warn("PDF jobs are saturated", "path", r.URL.Path, "error", err)
		w.Header().Set("Retry-After", "30")
		http.Error(w, "The server is busy, please try again shortly", http.StatusServiceUnavailable)
		return
	}
	switch {
	case r.Context().Err() != nil:
		// The client is gone, so nobody reads the response.
		slog.Info("request canceled", "path", r.URL.Path, "error", err)
	default:
		slog.Error("request failed", "method", r.Method, "path", r.URL.Path, "error", err)
		http.Error(w, "Internal Server Error", http.StatusInternalServerError)
	}
}

// logRequests logs each request's method, path, status, and duration.
func logRequests(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		start := time.Now()
		rec := &statusRecorder{ResponseWriter: w, status: http.StatusOK}
		next.ServeHTTP(rec, r)
		level := slog.LevelInfo
		if rec.status >= http.StatusInternalServerError {
			level = slog.LevelError
		}
		slog.Log(r.Context(), level, "request",
			"method", r.Method,
			"path", r.URL.Path,
			"status", rec.status,
			"duration", time.Since(start),
		)
	})
}

type statusRecorder struct {
	http.ResponseWriter
	status      int
	wroteHeader bool
}

func (r *statusRecorder) WriteHeader(status int) {
	if !r.wroteHeader {
		r.status = status
		r.wroteHeader = true
	}
	r.ResponseWriter.WriteHeader(status)
}

// ReadFrom keeps the underlying writer's sendfile path for file responses.
func (r *statusRecorder) ReadFrom(src io.Reader) (int64, error) {
	r.wroteHeader = true
	return io.Copy(r.ResponseWriter, src)
}

// Unwrap lets http.ResponseController reach the underlying writer.
func (r *statusRecorder) Unwrap() http.ResponseWriter { return r.ResponseWriter }
