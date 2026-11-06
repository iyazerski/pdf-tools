package main

import (
	"cmp"
	"fmt"
	"os"
	"strconv"
	"strings"
	"time"

	"github.com/iyazerski/pdf-tools/internal/server"
)

type config struct {
	username          string
	password          string
	sessionSecret     string
	bindAddr          string
	processTimeout    time.Duration
	uploadCacheTTL    time.Duration
	cookieSecure      server.CookieSecureMode
	trustProxyHeaders bool
}

// loadConfig reads the app's configuration from environment variables.
func loadConfig() (config, error) {
	var cfg config
	var err error
	if cfg.username, err = requiredEnv("APP_USERNAME"); err != nil {
		return cfg, err
	}
	if cfg.password, err = requiredEnv("APP_PASSWORD"); err != nil {
		return cfg, err
	}
	if cfg.sessionSecret, err = requiredEnv("SESSION_SECRET"); err != nil {
		return cfg, err
	}
	cfg.bindAddr = cmp.Or(os.Getenv("BIND_ADDR"), "0.0.0.0:8091")
	if cfg.processTimeout, err = envSeconds("EXTERNAL_PROCESS_TIMEOUT_SECS", 120); err != nil {
		return cfg, err
	}
	if cfg.uploadCacheTTL, err = envSeconds("UPLOAD_CACHE_TTL_SECS", 20*60); err != nil {
		return cfg, err
	}
	if cfg.cookieSecure, err = cookieSecureMode(cmp.Or(os.Getenv("COOKIE_SECURE"), "auto")); err != nil {
		return cfg, err
	}
	cfg.trustProxyHeaders = parseBoolLoose(os.Getenv("TRUST_PROXY_HEADERS"))
	return cfg, nil
}

func requiredEnv(key string) (string, error) {
	v := os.Getenv(key)
	if v == "" {
		return "", fmt.Errorf("%s must be set", key)
	}
	return v, nil
}

func envSeconds(key string, def int) (time.Duration, error) {
	v, ok := os.LookupEnv(key)
	if !ok {
		return time.Duration(def) * time.Second, nil
	}
	n, err := strconv.ParseUint(strings.TrimSpace(v), 10, 32)
	if err != nil {
		return 0, fmt.Errorf("%s must be a non-negative integer: %w", key, err)
	}
	return time.Duration(n) * time.Second, nil
}

func cookieSecureMode(v string) (server.CookieSecureMode, error) {
	switch strings.ToLower(strings.TrimSpace(v)) {
	case "auto":
		return server.CookieSecureAuto, nil
	case "true", "1", "on", "yes":
		return server.CookieSecureAlways, nil
	case "false", "0", "off", "no":
		return server.CookieSecureNever, nil
	}
	return 0, fmt.Errorf("COOKIE_SECURE must be one of: auto, true, false (got %q)", v)
}

func parseBoolLoose(v string) bool {
	switch strings.ToLower(strings.TrimSpace(v)) {
	case "1", "true", "on", "yes":
		return true
	}
	return false
}
