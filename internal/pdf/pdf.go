// Package pdf counts, assembles, compresses, and renders PDF files.
//
// Page counting and page assembly run in-process with pdfcpu. Lossy compression and page
// rendering run Ghostscript as a child process.
package pdf

import (
	"context"
	"errors"
	"fmt"
	"math"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"time"

	"github.com/pdfcpu/pdfcpu/pkg/api"
	"github.com/pdfcpu/pdfcpu/pkg/pdfcpu/model"
)

// maxConcurrentJobs caps parallel PDF jobs: each one can use a full CPU and hundreds of MB.
const maxConcurrentJobs = 2

// Merge quality bounds. LosslessQuality copies pages as they are, skipping the slow lossy
// Ghostscript pass.
const (
	MinQuality      = 10
	LosslessQuality = 100
)

// ErrInvalidPDF reports an input that can't be read as a PDF.
var ErrInvalidPDF = errors.New("invalid PDF")

// Processor runs PDF jobs with bounded concurrency and a per-job timeout.
type Processor struct {
	slots   chan struct{}
	timeout time.Duration
}

// NewProcessor returns a Processor that stops each job after timeout.
func NewProcessor(timeout time.Duration) *Processor {
	return &Processor{slots: make(chan struct{}, maxConcurrentJobs), timeout: timeout}
}

// PageRef selects one page (1-based) of an upload.
type PageRef struct {
	Upload string `json:"upload"`
	Page   int    `json:"page"`
}

// Input is a PDF on disk that Merge can take pages from.
type Input struct {
	Path  string
	Pages int
}

// PageCount returns the number of pages in the PDF at path, or an error wrapping
// ErrInvalidPDF when it can't be parsed.
func (p *Processor) PageCount(ctx context.Context, path string) (int, error) {
	var pages int
	err := p.do(ctx, "page count", func(ctx context.Context) error {
		f, err := os.Open(path)
		if err != nil {
			return err
		}
		defer f.Close()
		pages, err = api.PageCount(ctx, f, newConfig())
		if err != nil && ctx.Err() == nil {
			return fmt.Errorf("%w: %w", ErrInvalidPDF, err)
		}
		return err
	})
	return pages, err
}

// Merge builds one PDF from the pages in layout and returns its path.
//
// Every upload in layout must be in inputs, and every page must be in range. Below
// LosslessQuality, Ghostscript downsamples images to match quality (MinQuality or more).
// The result is written to dir, unless it would be identical to one whole input: then that
// input's path is returned as-is.
func (p *Processor) Merge(ctx context.Context, dir string, inputs map[string]Input, layout []PageRef, quality int) (string, error) {
	parts, err := p.collectParts(ctx, dir, inputs, layout)
	if err != nil {
		return "", err
	}
	if quality < LosslessQuality {
		// Ghostscript concatenates its inputs, so the parts need no merge pass of their own.
		return p.compress(ctx, dir, parts, quality)
	}
	if len(parts) == 1 {
		return parts[0], nil
	}

	out := filepath.Join(dir, "merged.pdf")
	err = p.do(ctx, "merge", func(ctx context.Context) error {
		conf := newConfig()
		// Without this, pdfcpu adds an outline entry named after each part file.
		conf.CreateBookmarks = false
		return api.MergeCreateFile(ctx, parts, out, false, conf)
	})
	return out, err
}

// collectParts returns one PDF per segment of layout, in order. Whole documents are used as
// they are, and the other segments are copied to new files in dir.
func (p *Processor) collectParts(ctx context.Context, dir string, inputs map[string]Input, layout []PageRef) ([]string, error) {
	var parts []string
	err := p.do(ctx, "collect pages", func(ctx context.Context) error {
		for i, seg := range segments(layout) {
			in, ok := inputs[seg.upload]
			if !ok {
				return fmt.Errorf("unknown upload %q", seg.upload)
			}
			if isWholeDocument(seg.pages, in.Pages) {
				parts = append(parts, in.Path)
				continue
			}
			part := filepath.Join(dir, fmt.Sprintf("part_%d.pdf", i))
			if err := api.CollectFile(ctx, in.Path, part, pageSelection(seg.pages), newConfig()); err != nil {
				return fmt.Errorf("pages of %q: %w", seg.upload, err)
			}
			parts = append(parts, part)
		}
		return nil
	})
	return parts, err
}

// newConfig returns the pdfcpu configuration for every job.
func newConfig() *model.Configuration {
	// pdfcpu's default configuration creates a config directory under $HOME, and the pod's
	// root filesystem is read-only.
	conf := model.NewStatelessConfiguration()
	// Copy pages as they are. Optimizing (deduplicating resources) makes jobs 5-10x slower
	// on text-heavy files, and the lossy Ghostscript pass already shrinks the output.
	conf.Optimize = false
	conf.OptimizeBeforeWriting = false
	return conf
}

// compress concatenates the PDFs in inputs with Ghostscript, downsampling and re-encoding
// images to match quality, and returns the new file's path in dir.
func (p *Processor) compress(ctx context.Context, dir string, inputs []string, quality int) (string, error) {
	out := filepath.Join(dir, "compressed.pdf")
	dpi, qFactor := qualityToGhostscriptParams(quality)
	// pdfwrite ignores -dJPEGQ, which only applies to the jpeg output devices. JPEG quality
	// comes from the QFactor of the image dictionaries instead. The ACS dictionaries apply
	// when gs picks JPEG on its own (photos), so flat images such as screenshots keep their
	// lossless encoding.
	jpegDict := fmt.Sprintf("<< /QFactor %s /Blend 1 /HSamples [2 1 1 2] /VSamples [2 1 1 2] >>",
		strconv.FormatFloat(qFactor, 'f', 2, 64))
	distillerParams := fmt.Sprintf("<< /ColorACSImageDict %[1]s /GrayACSImageDict %[1]s >> setdistillerparams", jpegDict)
	args := []string{
		"-sDEVICE=pdfwrite",
		// Keep gs's default compatibility level: forcing 1.4 makes it re-encode ICC-based
		// images (e.g. from macOS Preview) into files 100x+ larger and minutes slower.
		"-dDownsampleColorImages=true",
		"-dDownsampleGrayImages=true",
		"-dDownsampleMonoImages=true",
		"-dColorImageDownsampleType=/Bicubic",
		"-dGrayImageDownsampleType=/Bicubic",
		"-dColorImageResolution=" + strconv.Itoa(dpi),
		"-dGrayImageResolution=" + strconv.Itoa(dpi),
		"-dMonoImageResolution=600",
		// Without this, JPEG images that need no downsampling are copied as they are.
		"-dPassThroughJPEGImages=false",
		// gs reads the options after -c as PostScript, so the output file must come first.
		"-sOutputFile=" + out,
		"-c", distillerParams,
		"-f",
	}
	return out, p.runGhostscript(ctx, append(args, inputs...)...)
}

// RenderJPEG renders pages of the PDF at in to JPEG files at dpi.
//
// When page is 0, it renders every page, and out must contain %d, which Ghostscript
// replaces with the 1-based page number. Otherwise it renders only that page to out.
func (p *Processor) RenderJPEG(ctx context.Context, in string, page, dpi int, out string) error {
	args := []string{
		// JPEG is opaque with a white background, matching typical PDF viewers, and is far
		// smaller than PNG for scanned or photo pages.
		"-sDEVICE=jpeg",
		"-dJPEGQ=85",
		"-dTextAlphaBits=4",
		"-dGraphicsAlphaBits=4",
		"-r" + strconv.Itoa(dpi),
	}
	if page > 0 {
		args = append(args, "-dFirstPage="+strconv.Itoa(page), "-dLastPage="+strconv.Itoa(page))
	}
	args = append(args, "-sOutputFile="+out, in)
	return p.runGhostscript(ctx, args...)
}

func (p *Processor) runGhostscript(ctx context.Context, args ...string) error {
	return p.do(ctx, "ghostscript", func(ctx context.Context) error {
		args = append([]string{"-q", "-dSAFER", "-dNOPAUSE", "-dBATCH"}, args...)
		cmd := exec.CommandContext(ctx, "gs", args...)
		stderr := &prefixWriter{max: maxStderrBytes}
		cmd.Stderr = stderr
		// Don't wait forever for output pipes after the process is killed.
		cmd.WaitDelay = 5 * time.Second
		if err := cmd.Run(); err != nil {
			return fmt.Errorf("%w: %s", err, stderr)
		}
		return nil
	})
}

// do runs job once a slot is free, with the processor's timeout.
func (p *Processor) do(ctx context.Context, name string, job func(context.Context) error) error {
	select {
	case p.slots <- struct{}{}:
	case <-ctx.Done():
		return fmt.Errorf("%s: wait for a free slot: %w", name, ctx.Err())
	}
	defer func() { <-p.slots }()

	jobCtx, cancel := context.WithTimeout(ctx, p.timeout)
	defer cancel()
	err := job(jobCtx)
	if err == nil {
		return nil
	}
	if ctx.Err() == nil && errors.Is(jobCtx.Err(), context.DeadlineExceeded) {
		return fmt.Errorf("%s timed out after %s", name, p.timeout)
	}
	return fmt.Errorf("%s: %w", name, err)
}

// segment is a run of consecutive layout pages taken from the same upload.
type segment struct {
	upload string
	pages  []int
}

func segments(layout []PageRef) []segment {
	var segs []segment
	for _, ref := range layout {
		if n := len(segs); n > 0 && segs[n-1].upload == ref.Upload {
			segs[n-1].pages = append(segs[n-1].pages, ref.Page)
			continue
		}
		segs = append(segs, segment{upload: ref.Upload, pages: []int{ref.Page}})
	}
	return segs
}

// pageSelection converts pages to pdfcpu's page selection syntax, joining consecutive pages
// into ranges such as "1-3". pdfcpu's collect keeps the given order.
func pageSelection(pages []int) []string {
	var sel []string
	for i := 0; i < len(pages); {
		j := i
		for j+1 < len(pages) && pages[j+1] == pages[j]+1 {
			j++
		}
		if i == j {
			sel = append(sel, strconv.Itoa(pages[i]))
		} else {
			sel = append(sel, fmt.Sprintf("%d-%d", pages[i], pages[j]))
		}
		i = j + 1
	}
	return sel
}

func isWholeDocument(pages []int, total int) bool {
	if len(pages) != total {
		return false
	}
	for i, page := range pages {
		if page != i+1 {
			return false
		}
	}
	return true
}

// qualityToGhostscriptParams maps quality (MinQuality to LosslessQuality) linearly to an
// image resolution of 72 to 300 DPI and a JPEG QFactor of 2.4 to 0.15. Those QFactors are
// Acrobat Distiller's "minimum" and "maximum" JPEG quality, and lower means better.
func qualityToGhostscriptParams(quality int) (dpi int, qFactor float64) {
	t := float64(min(max(quality, MinQuality), LosslessQuality)-MinQuality) / (LosslessQuality - MinQuality)
	dpi = int(math.Round(72 + t*(300-72)))
	qFactor = 2.4 - t*(2.4-0.15)
	return dpi, qFactor
}

// maxStderrBytes caps how much of a child process's stderr goes into its error.
const maxStderrBytes = 512

// prefixWriter keeps the first max bytes written to it and discards the rest, so a chatty
// child process can't grow memory without bound.
type prefixWriter struct {
	buf       []byte
	max       int
	truncated bool
}

func (w *prefixWriter) Write(p []byte) (int, error) {
	n := min(len(p), w.max-len(w.buf))
	w.buf = append(w.buf, p[:n]...)
	w.truncated = w.truncated || n < len(p)
	// Report every byte as written: a short write would make exec stop draining the pipe.
	return len(p), nil
}

// String returns the kept bytes, marking dropped output with an ellipsis.
func (w *prefixWriter) String() string {
	if !w.truncated {
		return string(w.buf)
	}
	// The cut can split a multi-byte rune, so drop its partial bytes.
	return strings.ToValidUTF8(string(w.buf), "") + "…"
}
