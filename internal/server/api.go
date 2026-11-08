package server

import (
	"bytes"
	"encoding/json/v2"
	"errors"
	"io"
	"log/slog"
	"mime"
	"mime/multipart"
	"net/http"
	"os"
	"path/filepath"
	"strings"

	"github.com/iyazerski/pdf-tools/internal/pdf"
	"github.com/iyazerski/pdf-tools/internal/uploads"
)

const defaultQuality = 80

var pdfMagic = []byte("%PDF-")

type npagesResponse struct {
	Pages    int    `json:"pages"`
	UploadID string `json:"upload_id"`
}

// npages stores the uploaded PDF and returns its upload ID and page count.
func (s *Server) npages(w http.ResponseWriter, r *http.Request) (err error) {
	mr, err := r.MultipartReader()
	if err != nil {
		slog.Error("multipart parse failed", "error", err)
		return badRequest("Error parsing multipart/form-data request")
	}

	dir, err := os.MkdirTemp("", "upload-")
	if err != nil {
		return err
	}
	stored := false
	defer func() {
		if !stored {
			err = errors.Join(err, os.RemoveAll(dir))
		}
	}()

	path := filepath.Join(dir, "in.pdf")
	var fileName string
	for {
		part, err := mr.NextPart()
		if errors.Is(err, io.EOF) {
			return badRequest("Missing file")
		}
		if err != nil {
			return multipartError(err)
		}
		if part.FormName() != "file" {
			continue
		}
		if fileName, err = writePDFPart(part, path); err != nil {
			return err
		}
		break
	}

	pages, err := s.cfg.PDF.PageCount(r.Context(), path)
	if errors.Is(err, pdf.ErrInvalidPDF) {
		slog.Info("unreadable PDF", "file", fileName, "error", err)
		return badRequest("%s could not be read as a PDF", fileName)
	}
	if err != nil {
		return err
	}
	uploadID := s.cfg.Uploads.Put(dir, path, pages)
	stored = true
	slog.Info("computed page count", "pages", pages, "file", fileName)
	return writeJSON(w, npagesResponse{Pages: pages, UploadID: uploadID})
}

// writePDFPart streams a multipart PDF file to path, enforcing its type, size, and PDF
// header, and returns the file's name.
func writePDFPart(part *multipart.Part, path string) (fileName string, err error) {
	fileName = part.FileName()
	if fileName == "" {
		fileName = "file.pdf"
	}
	contentType, _, _ := strings.Cut(part.Header.Get("Content-Type"), ";")
	if contentType = strings.TrimSpace(contentType); contentType != "" && contentType != "application/pdf" {
		return "", badRequest("Only PDF files are allowed (got %s for %s)", contentType, fileName)
	}

	// Read one byte past the limit to detect files that are too large.
	src := io.LimitReader(part, maxFileBytes+1)
	header := make([]byte, len(pdfMagic))
	if _, err := io.ReadFull(src, header); err != nil || !bytes.Equal(header, pdfMagic) {
		if err != nil && !errors.Is(err, io.EOF) && !errors.Is(err, io.ErrUnexpectedEOF) {
			return "", multipartError(err)
		}
		return "", badRequest("%s does not look like a PDF", fileName)
	}

	f, err := os.Create(path)
	if err != nil {
		return "", err
	}
	defer func() { err = errors.Join(err, f.Close()) }()
	if _, err := f.Write(header); err != nil {
		return "", err
	}
	n, err := io.Copy(f, src)
	if err != nil {
		return "", multipartError(err)
	}
	if int64(len(header))+n > maxFileBytes {
		return "", badRequest("%s is too large (max %d MB)", fileName, maxFileBytes>>20)
	}
	return fileName, nil
}

// multipartError turns a failure to read the request body into a client error.
func multipartError(err error) error {
	if maxBytesErr, ok := errors.AsType[*http.MaxBytesError](err); ok {
		return maxBytesErr
	}
	return badRequest("Error reading multipart/form-data request: %v", err)
}

func (s *Server) deleteUpload(w http.ResponseWriter, r *http.Request) error {
	s.cfg.Uploads.Remove(r.PathValue("upload"))
	w.WriteHeader(http.StatusNoContent)
	return nil
}

type mergeRequest struct {
	Quality int       `json:"quality"`
	Layout  []pageRef `json:"layout"`
}

type pageRef struct {
	Upload string `json:"upload"`
	Page   int    `json:"page"`
}

// merge builds one PDF from the requested pages of earlier uploads.
func (s *Server) merge(w http.ResponseWriter, r *http.Request) error {
	if mediaType, _, _ := mime.ParseMediaType(r.Header.Get("Content-Type")); mediaType != "application/json" {
		return &httpError{status: http.StatusUnsupportedMediaType, msg: "Expected Content-Type: application/json"}
	}
	req := mergeRequest{Quality: defaultQuality}
	if err := json.UnmarshalRead(r.Body, &req); err != nil {
		if maxBytesErr, ok := errors.AsType[*http.MaxBytesError](err); ok {
			return maxBytesErr
		}
		slog.Info("invalid merge request", "error", err)
		return badRequest("Invalid JSON request body")
	}
	if len(req.Layout) == 0 {
		return badRequest("Layout is empty")
	}
	if len(req.Layout) > maxLayoutPages {
		return badRequest("Too many pages (max %d)", maxLayoutPages)
	}
	if req.Quality < pdf.MinQuality || req.Quality > pdf.LosslessQuality {
		return badRequest("Quality must be between %d and %d", pdf.MinQuality, pdf.LosslessQuality)
	}

	// Hold every upload for the whole merge, so the sweeper can't delete inputs midway.
	held := make(map[string]*uploads.Upload)
	defer func() {
		for _, upload := range held {
			s.cfg.Uploads.Release(upload)
		}
	}()
	inputs := make(map[string]pdf.Input)
	layout := make([]pdf.PageRef, len(req.Layout))
	for i, ref := range req.Layout {
		upload, ok := held[ref.Upload]
		if !ok {
			if len(held) == maxPDFs {
				return badRequest("Too many PDFs (max %d)", maxPDFs)
			}
			if upload, ok = s.cfg.Uploads.Get(ref.Upload); !ok {
				// The client still has the file, so it can upload it again and retry.
				w.Header().Set(missingUploadHeader, ref.Upload)
				return errUnknownUpload
			}
			held[ref.Upload] = upload
			inputs[ref.Upload] = pdf.Input{Path: upload.PDFPath, Pages: upload.Pages}
		}
		if ref.Page < 1 || ref.Page > upload.Pages {
			return badRequest("Invalid page %d (max %d)", ref.Page, upload.Pages)
		}
		layout[i] = pdf.PageRef{Upload: ref.Upload, Page: ref.Page}
	}

	dir, err := os.MkdirTemp("", "merge-")
	if err != nil {
		return err
	}
	defer os.RemoveAll(dir)

	out, err := s.cfg.PDF.Merge(r.Context(), dir, inputs, layout, req.Quality)
	if err != nil {
		return err
	}

	w.Header().Set("Content-Type", "application/pdf")
	w.Header().Set("Content-Disposition", `attachment; filename="merged.pdf"`)
	return serveFile(w, r, out)
}

// serveFile writes the file at path as the response body, using the content headers that
// are already set.
func serveFile(w http.ResponseWriter, r *http.Request, path string) error {
	f, err := os.Open(path)
	if err != nil {
		return err
	}
	defer f.Close()
	info, err := f.Stat()
	if err != nil {
		return err
	}
	http.ServeContent(w, r, "", info.ModTime(), f)
	return nil
}

func writeJSON(w http.ResponseWriter, v any) error {
	w.Header().Set("Content-Type", "application/json")
	return json.MarshalWrite(w, v)
}
