package server

import (
	"context"
	"fmt"
	"net/http"
	"os"
	"path/filepath"
	"strconv"

	"github.com/iyazerski/pdf-tools/internal/uploads"
)

const (
	thumbDPI = 40
	fullDPI  = 144

	// thumbBatchPages is how many thumbnails one Ghostscript run renders. Most of a run's
	// cost is startup and parsing, so batches are much faster than one run per page, while
	// the first thumbnails of a long document don't wait for all of its pages.
	thumbBatchPages = 20
)

// pageImage returns a JPEG of one page of an upload, rendering it on first use.
func (s *Server) pageImage(w http.ResponseWriter, r *http.Request) error {
	upload, ok := s.cfg.Uploads.Get(r.PathValue("upload"))
	if !ok {
		return badRequest("Unknown upload id")
	}
	defer s.cfg.Uploads.Release(upload)

	page, err := strconv.Atoi(r.PathValue("page"))
	if err != nil || page < 1 || page > upload.Pages {
		return badRequest("Invalid page (must be between 1 and %d)", upload.Pages)
	}

	var imagePath string
	switch kind := r.URL.Query().Get("kind"); kind {
	case "", "thumb":
		imagePath, err = s.thumbnail(r.Context(), upload, page)
	case "full":
		imagePath, err = s.fullPage(r.Context(), upload, page)
	default:
		return badRequest("Invalid kind %q (must be thumb or full)", kind)
	}
	if err != nil {
		return err
	}

	w.Header().Set("Content-Type", "image/jpeg")
	w.Header().Set("Cache-Control", "private, max-age=86400, immutable")
	return serveFile(w, r, imagePath)
}

// thumbnail returns the path of page's thumbnail, rendering its whole batch on first use.
func (s *Server) thumbnail(ctx context.Context, upload *uploads.Upload, page int) (string, error) {
	batch := (page - 1) / thumbBatchPages
	first := batch*thumbBatchPages + 1
	last := min(first+thumbBatchPages-1, upload.Pages)
	dir := filepath.Join(upload.Dir, "thumbs", strconv.Itoa(batch))
	err := upload.Render(ctx, "thumbs/"+strconv.Itoa(batch), func(ctx context.Context) error {
		if err := os.MkdirAll(dir, 0o700); err != nil {
			return err
		}
		return s.cfg.PDF.RenderJPEG(ctx, upload.PDFPath, first, last, thumbDPI, filepath.Join(dir, "%d.jpg"))
	})
	// Ghostscript numbers output files from 1 within each run.
	return filepath.Join(dir, fmt.Sprintf("%d.jpg", page-first+1)), err
}

// fullPage returns the path of page's full-size image, rendering it on first use.
func (s *Server) fullPage(ctx context.Context, upload *uploads.Upload, page int) (string, error) {
	path := filepath.Join(upload.Dir, fmt.Sprintf("full_%d.jpg", page))
	err := upload.Render(ctx, "full/"+strconv.Itoa(page), func(ctx context.Context) error {
		return s.cfg.PDF.RenderJPEG(ctx, upload.PDFPath, page, page, fullDPI, path)
	})
	return path, err
}
