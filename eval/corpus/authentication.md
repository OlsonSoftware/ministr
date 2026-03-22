# Authentication System

## Overview

The authentication subsystem implements a multi-layered security model combining OAuth 2.0 with PKCE for public clients and client credentials for service-to-service communication.

## Token Management

Access tokens are JSON Web Tokens (JWTs) signed with RS256 and contain claims for user identity, roles, and permissions. Tokens expire after 15 minutes by default.

Refresh tokens are opaque strings stored in the database with a 30-day expiration. They can be revoked individually or in bulk when a security incident is detected.

## Session Handling

Session management uses secure HTTP-only cookies with the SameSite=Strict attribute. CSRF protection is implemented via the double-submit cookie pattern.

All sessions are tracked server-side in a Redis-backed session store. Sessions include metadata such as IP address, user agent, and last activity timestamp.

## Rate Limiting

Rate limiting on the login endpoint prevents brute force attacks. After 5 failed attempts within a 10-minute window, the account enters exponential backoff:

- 1st lockout: 30 seconds
- 2nd lockout: 2 minutes
- 3rd lockout: 15 minutes
- 4th and beyond: 1 hour

## Audit Logging

All authentication events are captured in an immutable audit log, including:

- Login attempts (success and failure)
- Token refresh and revocation
- Session creation and destruction
- Permission changes
- IP address and geographic location derived from IP
