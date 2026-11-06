// Command pdf-tools serves a web app that merges, reorders, and compresses PDFs.
package main

import (
	"context"
	"embed"
	"errors"
	"fmt"
	"log/slog"
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
	// Stays below the pod's default 30s termination grace period.
	shutdownTimeout = 25 * time.Second
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

	httpServer := &http.Server{
		Addr:              cfg.bindAddr,
		Handler:           srv.Handler(),
		ReadHeaderTimeout: 10 * time.Second,
		IdleTimeout:       2 * time.Minute,
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

	shutdownCtx, cancel := context.WithTimeout(context.Background(), shutdownTimeout)
	defer cancel()
	if err := httpServer.Shutdown(shutdownCtx); err != nil {
		return fmt.Errorf("shut down: %w", err)
	}
	slog.Info("server stopped")
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
