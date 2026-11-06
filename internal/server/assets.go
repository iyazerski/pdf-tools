package server

import (
	"bytes"
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"io/fs"
	"net/http"
	"strings"
	"time"
)

const (
	authDataPlaceholder       = "{{AUTH_DATA}}"
	loginErrorDataPlaceholder = "{{LOGIN_ERROR_DATA}}"
	assetVersionPlaceholder   = "{{ASSET_VERSION}}"

	// app.js and styles.css URLs carry a content hash, so browsers can cache them forever.
	cacheImmutable = "public, max-age=31536000, immutable"
	cacheDaily     = "public, max-age=86400"
	// The page embeds the auth state, so it must be revalidated and never shared.
	cacheNoCache = "private, no-cache"
)

// assets holds the pre-rendered index pages and the static files, all in memory.
type assets struct {
	indexAuthed     staticAsset
	indexAnonymous  staticAsset
	indexLoginError staticAsset
	// static maps URL paths to files.
	static map[string]staticAsset
}

type staticAsset struct {
	body        []byte
	contentType string
	cache       string
	etag        string
}

func newStaticAsset(body []byte, contentType, cache string) staticAsset {
	sum := sha256.Sum256(body)
	return staticAsset{
		body:        body,
		contentType: contentType,
		cache:       cache,
		etag:        `"` + hex.EncodeToString(sum[:8]) + `"`,
	}
}

func (a staticAsset) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	w.Header().Set("Content-Type", a.contentType)
	w.Header().Set("Cache-Control", a.cache)
	// With an ETag, revalidations get a 304 instead of the whole body again.
	w.Header().Set("ETag", a.etag)
	http.ServeContent(w, r, "", time.Time{}, bytes.NewReader(a.body))
}

func loadAssets(fsys fs.FS) (*assets, error) {
	files := []struct {
		urlPath, file, contentType, cache string
	}{
		{"/static/app.js", "static/app.js", "text/javascript; charset=utf-8", cacheImmutable},
		{"/static/styles.css", "static/styles.css", "text/css; charset=utf-8", cacheImmutable},
		{"/favicon.svg", "static/favicon.svg", "image/svg+xml", cacheDaily},
		{"/og-image.svg", "static/og-image.svg", "image/svg+xml", cacheDaily},
		{"/robots.txt", "static/robots.txt", "text/plain; charset=utf-8", cacheDaily},
		{"/sitemap.xml", "static/sitemap.xml", "application/xml", cacheDaily},
	}
	a := &assets{static: make(map[string]staticAsset, len(files))}
	for _, f := range files {
		body, err := fs.ReadFile(fsys, f.file)
		if err != nil {
			return nil, err
		}
		a.static[f.urlPath] = newStaticAsset(body, f.contentType, f.cache)
	}

	template, err := fs.ReadFile(fsys, "templates/index.html")
	if err != nil {
		return nil, err
	}
	// Fail fast on template drift: it's easy to remove placeholders by accident.
	for _, placeholder := range []string{authDataPlaceholder, loginErrorDataPlaceholder, assetVersionPlaceholder} {
		if !strings.Contains(string(template), placeholder) {
			return nil, fmt.Errorf("templates/index.html is missing %s", placeholder)
		}
	}

	h := sha256.New()
	h.Write(a.static["/static/app.js"].body)
	h.Write(a.static["/static/styles.css"].body)
	version := hex.EncodeToString(h.Sum(nil)[:6])

	render := func(authed, loginError string) staticAsset {
		page := strings.NewReplacer(
			assetVersionPlaceholder, version,
			authDataPlaceholder, authed,
			loginErrorDataPlaceholder, loginError,
		).Replace(string(template))
		return newStaticAsset([]byte(page), "text/html; charset=utf-8", cacheNoCache)
	}
	a.indexAuthed = render("1", "0")
	a.indexAnonymous = render("0", "0")
	a.indexLoginError = render("0", "1")
	return a, nil
}
