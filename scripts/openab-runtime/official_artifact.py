#!/usr/bin/env python3
"""Fetch the pinned official openab-pty Linux binary out of the GHCR image.

The anonymous registry token lives only in memory and in the Authorization
header sent to ghcr.io; it is never printed, persisted or forwarded to a
redirected host. Every artifact is size/digest verified and the pinned layer
tar is streamed so only the single expected regular member is read.
"""
import hashlib
import io
import json
import platform
import re
import tarfile
import urllib.error
import urllib.parse
import urllib.request

REGISTRY = 'ghcr.io'
REPOSITORY = 'openabdev/openab-pty'
TAG = 'native-b82a366'
UPSTREAM_COMMIT = 'b82a36620a76c84a67bc6a7ab316cbc60a30e1a9'
IMAGE_DIGEST = 'sha256:6a1606da35f113eb5d41684a5eec3f7751b2706c90997853c79e1093a5a5b1c2'
CONFIG_DIGEST = 'sha256:7b9b5bd505a5e2335cb3b3eaad1937bf96d756321bfdb38d80a225ab08ab9532'
LAYER_DIGEST = 'sha256:c00d66b8193e3c230e4e17b2832b2dde974d52155a8443bf3193cddb35bf9775'
LAYER_BYTES = 2154101
MEMBER = 'usr/local/bin/openab-pty'
BINARY_SHA256 = '833b574e47d0f3017642a62f119375c091ead4553fce505023001ff4090bf3b6'
BINARY_BYTES = 4861736
IMAGE = REGISTRY + '/' + REPOSITORY + ':' + TAG
MANIFEST_ACCEPT = ('application/vnd.oci.image.manifest.v1+json,'
                   'application/vnd.docker.distribution.manifest.v2+json')
MAX_MANIFEST = 256 * 1024
MAX_CONFIG = 64 * 1024
MAX_TOKEN_REPLY = 16 * 1024


class Refusal(RuntimeError):
    pass


def require(condition, message):
    if not condition:
        raise Refusal(message)


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def default_opener():
    return urllib.request.build_opener(NoRedirect())


def digest(data):
    return hashlib.sha256(data).hexdigest()


def parsed(raw, what):
    try:
        return json.loads(raw)
    except ValueError:
        raise Refusal(what + ' is not valid JSON')


def get(opener, url, token='', accept='', limit=MAX_MANIFEST, redirects=4):
    headers = {}
    if accept:
        headers['Accept'] = accept
    if token:
        headers['Authorization'] = 'Bearer ' + token
    for _ in range(redirects):
        request = urllib.request.Request(url, headers=headers)
        try:
            response = opener.open(request, timeout=60)
        except urllib.error.HTTPError as error:
            location = error.headers.get('Location') if error.headers else None
            if error.code in (301, 302, 303, 307, 308) and location:
                url = urllib.parse.urljoin(url, location)
                parts = urllib.parse.urlsplit(url)
                require(parts.scheme == 'https' and parts.hostname and
                        not parts.username and not parts.password,
                        'Unsafe registry redirect target')
                # The registry bearer is never forwarded to a redirected host.
                headers.pop('Authorization', None)
                continue
            raise Refusal('Registry request failed: HTTP ' + str(error.code))
        except (OSError, ValueError) as error:
            raise Refusal('Registry request failed: ' + type(error).__name__)
        try:
            data = response.read(limit + 1)
        finally:
            response.close()
        require(len(data) <= limit, 'Registry response exceeded bound')
        return data
    raise Refusal('Registry redirect limit exceeded')


def token(opener):
    url = ('https://' + REGISTRY + '/token?service=' + REGISTRY +
           '&scope=repository:' + REPOSITORY + ':pull')
    value = parsed(get(opener, url, limit=MAX_TOKEN_REPLY), 'Token reply')
    require(isinstance(value, dict), 'Token reply must be an object')
    secret = value.get('token')
    require(isinstance(secret, str) and
            re.fullmatch(r'[A-Za-z0-9._~+/=-]{1,8192}', secret),
            'Invalid anonymous registry token')
    return secret


def extract_member(layer):
    require(len(layer) == LAYER_BYTES and digest(layer) == LAYER_DIGEST[7:],
            'Pinned layer size/digest mismatch')
    binary = None
    with tarfile.open(fileobj=io.BytesIO(layer), mode='r|gz') as tar:
        for entry in tar:
            if entry.name in (MEMBER, './' + MEMBER):
                require(binary is None, 'Duplicate runtime member in pinned layer')
                require(entry.isreg() and entry.size == BINARY_BYTES,
                        'Runtime member is not the expected regular file')
                stream = tar.extractfile(entry)
                require(stream is not None, 'Runtime member is not readable')
                binary = stream.read(BINARY_BYTES + 1)
    require(binary is not None, 'Runtime member missing from pinned layer')
    require(len(binary) == BINARY_BYTES and digest(binary) == BINARY_SHA256,
            'Pinned runtime binary size/digest mismatch')
    return binary


def fetch(opener=None):
    require(platform.machine() in ('x86_64', 'amd64'),
            'The pinned official artifact supports x86_64 Linux Hosts only; '
            'supply an unmodified official binary via --binary/--sha256 on other architectures')
    opener = opener or default_opener()
    auth = token(opener)
    manifest_raw = get(opener, 'https://' + REGISTRY + '/v2/' + REPOSITORY +
                       '/manifests/' + IMAGE_DIGEST, auth, accept=MANIFEST_ACCEPT,
                       limit=MAX_MANIFEST)
    require(digest(manifest_raw) == IMAGE_DIGEST[7:], 'Pinned image manifest digest mismatch')
    manifest = parsed(manifest_raw, 'Image manifest')
    require(isinstance(manifest, dict), 'Image manifest must be an object')
    config = manifest.get('config') or {}
    layers = manifest.get('layers') or []
    require(config.get('digest') == CONFIG_DIGEST, 'Pinned image config digest mismatch')
    matching = [layer for layer in layers if layer.get('digest') == LAYER_DIGEST]
    require(len(matching) == 1 and matching[0].get('size') == LAYER_BYTES,
            'Pinned runtime layer missing from image manifest')
    config_raw = get(opener, 'https://' + REGISTRY + '/v2/' + REPOSITORY +
                     '/blobs/' + CONFIG_DIGEST, auth, limit=MAX_CONFIG)
    require(digest(config_raw) == CONFIG_DIGEST[7:], 'Pinned image config blob mismatch')
    image_config = parsed(config_raw, 'Image config')
    require(isinstance(image_config, dict) and image_config.get('architecture') == 'amd64' and
            image_config.get('os') == 'linux', 'Pinned official image is not linux/amd64')
    layer = get(opener, 'https://' + REGISTRY + '/v2/' + REPOSITORY + '/blobs/' +
                LAYER_DIGEST, auth, limit=LAYER_BYTES)
    return extract_member(layer)


def provenance():
    return {'kind': 'official-ghcr', 'image': IMAGE, 'upstream_commit': UPSTREAM_COMMIT,
            'image_digest': IMAGE_DIGEST, 'config_digest': CONFIG_DIGEST,
            'layer_digest': LAYER_DIGEST, 'layer_bytes': LAYER_BYTES, 'member': MEMBER,
            'binary_sha256': BINARY_SHA256, 'binary_bytes': BINARY_BYTES}
