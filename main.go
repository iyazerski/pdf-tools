// Command pdf-tools serves a web app that merges, reorders, and compresses PDFs.
package main

import (
	"context"
	"embed"
	"errors"
	"fmt"
	"log/slog"
	"net"
	"net/http"
	"os"
	"os/signal"
	"sync"
	"syscall"
	"time"

	"github.com/joho/godotenv"

	"github.com/iyazerski/pdf-tools/internal/pdf"
	"github.com/iyazerski/pdf-tools/internal/server"
	"github.com/iyazerski/pdf-tools/internal/session"
	"github.com/iyazerski/pdf-tools/internal/uploads"
)

const (
	sessionTTL = 24 * time.Hour
	// Keeps worst-case disk use (entries x max file size plus one merge) under the 2Gi
	// emptyDir.
	uploadCacheMaxEntries = 15
	// Shutdown waits drainTimeout for requests to finish, then cancels them and waits
	// cancelTimeout more for them to stop. Together they stay below the pod's default 30s
	// termination grace period.
	drainTimeout  = 20 * time.Second
	cancelTimeout = 7 * time.Second

	// Uploads of maxFileBytes must fit in readTimeout on slow links. writeTimeout runs from
	// the end of the request headers, so it also covers reading the body and a PDF job.
	readTimeout  = 5 * time.Minute
	writeTimeout = 10 * time.Minute
)

// Assets are compiled into the binary: no file I/O per request and nothing to copy at deploy.
//
//go:embed templates/index.html static
var assetsFS embed.FS

func main() {
	if err := run(); err != nil {
		slog.Error("server failed", "error", err)
		os.Exit(1)
	}
}

func run() error {
	if err := setupLogging(); err != nil {
		return err
	}
	// A missing .env file is fine: it's only a local development convenience.
	if err := godotenv.Load(); err != nil && !errors.Is(err, os.ErrNotExist) {
		return fmt.Errorf("load .env: %w", err)
	}
	cfg, err := loadConfig()
	if err != nil {
		return err
	}

	secret := []byte(cfg.sessionSecret)
	store := uploads.NewStore(cfg.uploadCacheTTL, uploadCacheMaxEntries)
	srv, err := server.New(server.Config{
		Credentials:       session.NewCredentials(cfg.username, cfg.password, secret),
		Signer:            session.NewSigner(secret, sessionTTL),
		CookieSecure:      cfg.cookieSecure,
		TrustProxyHeaders: cfg.trustProxyHeaders,
		Uploads:           store,
		PDF:               pdf.NewProcessor(cfg.processTimeout, cfg.maxPDFJobs),
		Assets:            assetsFS,
	})
	if err != nil {
		return err
	}

	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()

	var wg sync.WaitGroup
	sweepCtx, stopSweep := context.WithCancel(context.Background())
	// Stopping the sweeper deletes all uploads, so it must outlive in-flight requests.
	defer func() {
		stopSweep()
		wg.Wait()
	}()
	wg.Go(func() { store.Run(sweepCtx) })

	// Canceling requestsCtx cancels every request, which stops their PDF jobs.
	requestsCtx, cancelRequests := context.WithCancel(context.Background())
	defer cancelRequests()
	httpServer := &http.Server{
		Addr:              cfg.bindAddr,
		Handler:           srv.Handler(),
		ReadHeaderTimeout: 10 * time.Second,
		ReadTimeout:       readTimeout,
		WriteTimeout:      writeTimeout,
		IdleTimeout:       2 * time.Minute,
		BaseContext:       func(net.Listener) context.Context { return requestsCtx },
	}
	serveErr := make(chan error, 1)
	wg.Go(func() {
		slog.Info("starting server", "bind", cfg.bindAddr)
		serveErr <- httpServer.ListenAndServe()
	})

	select {
	case err := <-serveErr:
		return err
	case <-ctx.Done():
		slog.Info("shutdown signal received")
	}

	if err := shutdown(httpServer, cancelRequests, drainTimeout, cancelTimeout); err != nil {
		return err
	}
	slog.Info("server stopped")
	return nil
}

// shutdown stops srv and returns once its handlers are done. Handlers that outlast drain
// are canceled with cancelRequests and get cancelWait more to return. Handlers must be done
// before uploads are deleted, or running jobs lose their input files.
func shutdown(srv *http.Server, cancelRequests context.CancelFunc, drain, cancelWait time.Duration) error {
	drainCtx, cancelDrain := context.WithTimeout(context.Background(), drain)
	defer cancelDrain()
	err := srv.Shutdown(drainCtx)
	if !errors.Is(err, context.DeadlineExceeded) {
		return err
	}

	slog.Warn("requests still running after the drain timeout, canceling them")
	cancelRequests()
	waitCtx, cancelWaitCtx := context.WithTimeout(context.Background(), cancelWait)
	defer cancelWaitCtx()
	// Shutdown again only waits: its listeners are already closed.
	if err := srv.Shutdown(waitCtx); err != nil {
		return errors.Join(fmt.Errorf("wait for canceled requests: %w", err), srv.Close())
	}
	return nil
}

// setupLogging logs plain text to stdout at LOG_LEVEL (default info). Logs go to kubectl,
// not a terminal, so there are no colors.
func setupLogging() error {
	var level slog.Level
	if v := os.Getenv("LOG_LEVEL"); v != "" {
		if err := level.UnmarshalText([]byte(v)); err != nil {
			return fmt.Errorf("LOG_LEVEL: %w", err)
		}
	}
	slog.SetDefault(slog.New(slog.NewTextHandler(os.Stdout, &slog.HandlerOptions{Level: level})))
	return nil
}
