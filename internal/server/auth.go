package server

import (
	"net/http"
	"strings"
	"time"
)

func (s *Server) index(w http.ResponseWriter, r *http.Request) error {
	switch {
	case s.authedUser(r) != "":
		s.assets.indexAuthed.ServeHTTP(w, r)
	case r.URL.Query().Has("login_error"):
		s.assets.indexLoginError.ServeHTTP(w, r)
	default:
		s.assets.indexAnonymous.ServeHTTP(w, r)
	}
	return nil
}

func (s *Server) login(w http.ResponseWriter, r *http.Request) error {
	username, password := r.PostFormValue("username"), r.PostFormValue("password")
	if !s.cfg.Credentials.Matches(username, password) {
		http.Redirect(w, r, "/?login_error=1", http.StatusSeeOther)
		return nil
	}

	http.SetCookie(w, &http.Cookie{
		Name:     sessionCookieName,
		Value:    s.cfg.Signer.Issue(username, time.Now()),
		Path:     "/",
		HttpOnly: true,
		SameSite: http.SameSiteLaxMode,
		Secure:   s.cookieSecure(r),
	})
	http.Redirect(w, r, "/", http.StatusSeeOther)
	return nil
}

func (s *Server) logout(w http.ResponseWriter, r *http.Request) error {
	http.SetCookie(w, &http.Cookie{
		Name:     sessionCookieName,
		Path:     "/",
		MaxAge:   -1,
		HttpOnly: true,
		SameSite: http.SameSiteLaxMode,
		Secure:   s.cookieSecure(r),
	})
	http.Redirect(w, r, "/", http.StatusSeeOther)
	return nil
}

// authedUser returns the username of a valid session, or "" if there is none.
func (s *Server) authedUser(r *http.Request) string {
	cookie, err := r.Cookie(sessionCookieName)
	if err != nil {
		return ""
	}
	username, _ := s.cfg.Signer.Verify(cookie.Value, time.Now())
	return username
}

func (s *Server) requireAuth(r *http.Request) error {
	if s.authedUser(r) == "" {
		return errUnauthorized
	}
	return nil
}

func (s *Server) cookieSecure(r *http.Request) bool {
	switch s.cfg.CookieSecure {
	case CookieSecureAlways:
		return true
	case CookieSecureNever:
		return false
	default:
		return s.cfg.TrustProxyHeaders && forwardedProtoIsHTTPS(r.Header)
	}
}

func forwardedProtoIsHTTPS(h http.Header) bool {
	proto, _, _ := strings.Cut(h.Get("X-Forwarded-Proto"), ",")
	if strings.EqualFold(strings.TrimSpace(proto), "https") {
		return true
	}
	return strings.Contains(strings.ToLower(h.Get("Forwarded")), "proto=https")
}
