package server

import (
	"net/http"
	"net/netip"
	"slices"
	"strconv"
	"strings"
	"sync"
	"time"

	"golang.org/x/time/rate"
)

// pruneInterval is how often ipLimiter drops limiters of clients that have fully recovered.
const pruneInterval = time.Minute

// ipLimiter is a token bucket rate limiter per client IP.
type ipLimiter struct {
	every time.Duration
	burst int

	mu        sync.Mutex
	limiters  map[netip.Addr]*rate.Limiter
	lastPrune time.Time
}

func newIPLimiter(every time.Duration, burst int) *ipLimiter {
	return &ipLimiter{every: every, burst: burst, limiters: make(map[netip.Addr]*rate.Limiter)}
}

func (l *ipLimiter) allow(ip netip.Addr, now time.Time) bool {
	l.mu.Lock()
	defer l.mu.Unlock()

	// A limiter with a full bucket behaves like a new one, so dropping it changes nothing
	// and keeps the map bounded by the number of recently active clients.
	if now.Sub(l.lastPrune) >= pruneInterval {
		for key, lim := range l.limiters {
			if lim.TokensAt(now) >= float64(l.burst) {
				delete(l.limiters, key)
			}
		}
		l.lastPrune = now
	}

	lim, ok := l.limiters[ip]
	if !ok {
		lim = rate.NewLimiter(rate.Every(l.every), l.burst)
		l.limiters[ip] = lim
	}
	return lim.AllowN(now, 1)
}

func (s *Server) rateLimited(next handlerFunc) handlerFunc {
	return func(w http.ResponseWriter, r *http.Request) error {
		ip, err := s.clientIP(r)
		if err != nil {
			return err
		}
		if !s.loginLimiter.allow(ip, time.Now()) {
			w.Header().Set("Retry-After", strconv.Itoa(int(s.loginLimiter.every.Seconds())))
			return &httpError{status: http.StatusTooManyRequests, msg: "Too Many Requests"}
		}
		return next(w, r)
	}
}

// clientIP returns the rightmost valid X-Forwarded-For address when proxy headers are
// trusted (the one the trusted proxy appended), and the peer address otherwise.
func (s *Server) clientIP(r *http.Request) (netip.Addr, error) {
	if s.cfg.TrustProxyHeaders {
		parts := strings.Split(r.Header.Get("X-Forwarded-For"), ",")
		for _, part := range slices.Backward(parts) {
			if ip, err := netip.ParseAddr(strings.TrimSpace(part)); err == nil {
				return ip, nil
			}
		}
	}
	addrPort, err := netip.ParseAddrPort(r.RemoteAddr)
	if err != nil {
		return netip.Addr{}, err
	}
	return addrPort.Addr(), nil
}
