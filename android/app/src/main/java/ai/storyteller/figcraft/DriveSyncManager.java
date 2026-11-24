package ai.storyteller.figcraft;

import android.app.Activity;
import android.content.Intent;
import android.net.Uri;
import android.util.Base64;

import org.json.JSONArray;
import org.json.JSONObject;

import java.io.ByteArrayOutputStream;
import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.HttpURLConnection;
import java.net.InetAddress;
import java.net.ServerSocket;
import java.net.Socket;
import java.net.URL;
import java.net.URLEncoder;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.security.SecureRandom;
import java.util.Scanner;
import java.util.UUID;

/**
 * Lightweight Google Drive REST v3 + OAuth 2.0 Loopback PKCE client for FigCraft Mobile.
 * Uses only Android standard library APIs (no external Google Play Services dependency).
 */
final class DriveSyncManager {
    private static final String DRIVE_SCOPE = "https://www.googleapis.com/auth/drive.file https://www.googleapis.com/auth/userinfo.email";
    private static final String FOLDER_NAME = "FigCraft";

    interface Listener {
        void onAuthChanged(boolean signedIn, String email);
        void onIndexUpdated(String indexJson);
        void onStatus(String message);
        void onFileReadyToOpen(String localPath, String displayName);
        void onCredentialsImported(String clientId, String clientSecret);
    }

    private final Activity activity;
    private final File filesDir;
    private final File cacheDir;
    private final File authFile;
    private final File indexFile;
    private final File docMapFile;
    private final Listener listener;

    private String clientId = "";
    private String clientSecret = "";
    private String accessToken = "";
    private String refreshToken = "";
    private long expiresAtMs = 0L;
    private String userEmail = "";
    private String folderId = "";

    DriveSyncManager(Activity activity, Listener listener) {
        this.activity = activity;
        this.filesDir = activity.getFilesDir();
        this.cacheDir = new File(filesDir, "drive_cache");
        if (!cacheDir.exists()) {
            //noinspection ResultOfMethodCallIgnored
            cacheDir.mkdirs();
        }
        this.authFile = new File(filesDir, "drive_auth.json");
        this.indexFile = new File(cacheDir, "drive_index.json");
        this.docMapFile = new File(cacheDir, "drive_doc_map.json");
        this.listener = listener;
        loadAuth();
    }

    synchronized void emitInitialState() {
        listener.onAuthChanged(isSignedIn(), userEmail);
        if (indexFile.exists()) {
            try {
                listener.onIndexUpdated(readText(indexFile));
            } catch (Exception ignored) {
            }
        }
    }

    synchronized boolean isSignedIn() {
        return !refreshToken.isEmpty() || (!accessToken.isEmpty() && System.currentTimeMillis() < expiresAtMs);
    }

    void importCredentialsJson(InputStream in) {
        try {
            if (in == null) {
                listener.onStatus("JSON 파일을 열 수 없습니다.");
                return;
            }
            String text = new String(readStream(in), StandardCharsets.UTF_8);
            JSONObject root = new JSONObject(text);
            JSONObject section = root.optJSONObject("installed");
            if (section == null) section = root.optJSONObject("web");
            if (section == null) section = root;
            String cid = section.optString("client_id", "").trim();
            String csec = section.optString("client_secret", "").trim();
            if (cid.isEmpty()) {
                listener.onStatus("올바른 Google OAuth JSON 파일이 아닙니다 (client_id 없음).");
                return;
            }
            synchronized (this) {
                this.clientId = cid;
                this.clientSecret = csec;
                saveAuth();
            }
            listener.onCredentialsImported(cid, csec);
            listener.onStatus("OAuth JSON 불러오기 완료! 이제 'Google 로그인'을 누르세요.");
        } catch (Exception e) {
            listener.onStatus("JSON 파싱 오류: " + e.getMessage());
        }
    }

    void startSignIn(String inputClientId, String inputClientSecret) {
        final String cid = inputClientId != null ? inputClientId.trim() : "";
        final String csec = inputClientSecret != null ? inputClientSecret.trim() : "";
        if (cid.isEmpty()) {
            listener.onStatus("먼저 JSON 파일을 불러오거나 OAuth Client ID를 입력하세요.");
            return;
        }
        synchronized (this) {
            this.clientId = cid;
            this.clientSecret = csec;
            saveAuth();
        }

        new Thread(() -> {
            ServerSocket server = null;
            try {
                byte[] verifierBytes = new byte[32];
                new SecureRandom().nextBytes(verifierBytes);
                String codeVerifier = Base64.encodeToString(verifierBytes, Base64.URL_SAFE | Base64.NO_WRAP | Base64.NO_PADDING);
                MessageDigest md = MessageDigest.getInstance("SHA-256");
                byte[] digest = md.digest(codeVerifier.getBytes(StandardCharsets.US_ASCII));
                String codeChallenge = Base64.encodeToString(digest, Base64.URL_SAFE | Base64.NO_WRAP | Base64.NO_PADDING);
                String state = UUID.randomUUID().toString();

                server = new ServerSocket(0, 1, InetAddress.getByName("127.0.0.1"));
                server.setSoTimeout(180_000);
                int port = server.getLocalPort();
                String redirectUri = "http://127.0.0.1:" + port;

                String authUrl = "https://accounts.google.com/o/oauth2/v2/auth"
                        + "?client_id=" + enc(cid)
                        + "&redirect_uri=" + enc(redirectUri)
                        + "&response_type=code"
                        + "&scope=" + enc(DRIVE_SCOPE)
                        + "&code_challenge=" + enc(codeChallenge)
                        + "&code_challenge_method=S256"
                        + "&state=" + enc(state)
                        + "&access_type=offline"
                        + "&prompt=consent";

                activity.runOnUiThread(() -> {
                    try {
                        Intent browser = new Intent(Intent.ACTION_VIEW, Uri.parse(authUrl));
                        activity.startActivity(browser);
                    } catch (Exception e) {
                        listener.onStatus("브라우저를 열 수 없습니다: " + e.getMessage());
                    }
                });
                listener.onStatus("브라우저에서 Google 로그인을 진행하세요...");

                try (Socket client = server.accept()) {
                    Scanner sc = new Scanner(client.getInputStream(), "UTF-8");
                    String firstLine = sc.hasNextLine() ? sc.nextLine() : "";
                    String code = extractQueryParam(firstLine, "code");
                    String err = extractQueryParam(firstLine, "error");

                    String html = (code != null && !code.isEmpty())
                            ? "<html><body style='font-family:sans-serif;text-align:center;padding:48px'><h2>FigCraft Mobile 로그인 완료</h2><p>이 창을 닫고 FigCraft 앱으로 돌아가세요.</p></body></html>"
                            : "<html><body style='font-family:sans-serif;text-align:center;padding:48px'><h2>로그인 취소/오류</h2><p>" + (err != null ? err : "unknown") + "</p></body></html>";
                    byte[] body = html.getBytes(StandardCharsets.UTF_8);
                    OutputStream out = client.getOutputStream();
                    out.write(("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: " + body.length + "\r\nConnection: close\r\n\r\n").getBytes(StandardCharsets.US_ASCII));
                    out.write(body);
                    out.flush();

                    if (code == null || code.isEmpty()) {
                        listener.onStatus("Google 로그인이 취소되었거나 실패했습니다.");
                        return;
                    }

                    String tokenBody = "code=" + enc(code)
                            + "&client_id=" + enc(cid)
                            + (csec.isEmpty() ? "" : ("&client_secret=" + enc(csec)))
                            + "&code_verifier=" + enc(codeVerifier)
                            + "&redirect_uri=" + enc(redirectUri)
                            + "&grant_type=authorization_code";
                    JSONObject tok = httpJson("POST", "https://oauth2.googleapis.com/token", null, "application/x-www-form-urlencoded", tokenBody.getBytes(StandardCharsets.UTF_8));
                    synchronized (DriveSyncManager.this) {
                        accessToken = tok.optString("access_token", "");
                        if (tok.has("refresh_token")) {
                            refreshToken = tok.optString("refresh_token", refreshToken);
                        }
                        long expiresIn = tok.optLong("expires_in", 3600L);
                        expiresAtMs = System.currentTimeMillis() + Math.max(60L, expiresIn - 60L) * 1000L;
                        userEmail = fetchEmail(accessToken);
                        folderId = "";
                        saveAuth();
                    }
                    listener.onAuthChanged(true, userEmail);
                    listener.onStatus("Google Drive 연결 완료 (" + (userEmail.isEmpty() ? "로그인됨" : userEmail) + ")");
                    syncAndListBlocking();
                }
            } catch (Exception e) {
                listener.onStatus("Google 로그인 오류: " + e.getMessage());
            } finally {
                if (server != null) {
                    try {
                        server.close();
                    } catch (Exception ignored) {
                    }
                }
            }
        }, "figcraft-drive-oauth").start();
    }

    synchronized void signOut() {
        accessToken = "";
        refreshToken = "";
        expiresAtMs = 0L;
        userEmail = "";
        folderId = "";
        saveAuth();
        listener.onAuthChanged(false, "");
        listener.onStatus("Google Drive 연결이 해제되었습니다.");
    }

    void syncAndListAsync() {
        new Thread(this::syncAndListBlocking, "figcraft-drive-sync").start();
    }

    private synchronized String getMappedFileId(String docUid) {
        if (docUid == null || docUid.isEmpty() || !docMapFile.exists()) return "";
        try {
            JSONObject map = new JSONObject(readText(docMapFile));
            return map.optString(docUid, "");
        } catch (Exception ignored) {
            return "";
        }
    }

    private synchronized void putMappedFileId(String docUid, String fileId) {
        if (docUid == null || docUid.isEmpty() || fileId == null || fileId.isEmpty()) return;
        try {
            JSONObject map = docMapFile.exists() ? new JSONObject(readText(docMapFile)) : new JSONObject();
            map.put(docUid, fileId);
            writeBytes(docMapFile, map.toString().getBytes(StandardCharsets.UTF_8));
        } catch (Exception ignored) {
        }
    }

    private synchronized void removeMappedFileId(String fileId) {
        if (fileId == null || fileId.isEmpty() || !docMapFile.exists()) return;
        try {
            JSONObject map = new JSONObject(readText(docMapFile));
            JSONObject next = new JSONObject();
            java.util.Iterator<String> keys = map.keys();
            while (keys.hasNext()) {
                String k = keys.next();
                String v = map.optString(k, "");
                if (!fileId.equals(v)) next.put(k, v);
            }
            writeBytes(docMapFile, next.toString().getBytes(StandardCharsets.UTF_8));
        } catch (Exception ignored) {
        }
    }

    void uploadCanvasAsync(String docUid, String rawName, String localFigPath, String localThumbPath) {
        new Thread(() -> {
            try {
                if (!ensureAccessToken()) {
                    return;
                }
                String fid = ensureFolderId();
                if (fid.isEmpty()) {
                    listener.onStatus("Drive FigCraft 폴더를 만들지 못했습니다.");
                    return;
                }
                String safeName = sanitizeCanvasName(rawName);
                String figFileName = safeName.endsWith(".figcraft") ? safeName : (safeName + ".figcraft");
                File localFig = new File(localFigPath);
                if (!localFig.exists()) return;
                byte[] figBytes = readBytes(localFig);

                File cachedFig = new File(cacheDir, figFileName);
                writeBytes(cachedFig, figBytes);
                String cachedThumbPath = "";
                if (localThumbPath != null && !localThumbPath.isEmpty()) {
                    File srcThumb = new File(localThumbPath);
                    if (srcThumb.exists()) {
                        File dstThumb = new File(cacheDir, figFileName + ".thumb.png");
                        writeBytes(dstThumb, readBytes(srcThumb));
                        cachedThumbPath = dstThumb.getAbsolutePath();
                    }
                }

                // Prefer 1:1 docUid mapping so renaming a canvas renames & updates the existing Drive file without duplicates
                String existingId = getMappedFileId(docUid);
                if (existingId.isEmpty()) {
                    existingId = findFileInFolder(fid, figFileName);
                }

                JSONObject uploaded = null;
                if (!existingId.isEmpty()) {
                    try {
                        JSONObject patchMeta = new JSONObject();
                        patchMeta.put("name", figFileName);
                        uploaded = uploadMultipart(
                                "PATCH",
                                "https://www.googleapis.com/upload/drive/v3/files/" + enc(existingId) + "?uploadType=multipart&fields=id,name,modifiedTime",
                                patchMeta,
                                "application/octet-stream",
                                figBytes
                        );
                    } catch (Exception patchErr) {
                        // If remote file was deleted externally (404), fall back to creating a new file
                        existingId = "";
                    }
                }
                if (uploaded == null) {
                    JSONObject createMeta = new JSONObject();
                    createMeta.put("name", figFileName);
                    JSONArray parents = new JSONArray();
                    parents.put(fid);
                    createMeta.put("parents", parents);
                    uploaded = uploadMultipart(
                            "POST",
                            "https://www.googleapis.com/upload/drive/v3/files?uploadType=multipart&fields=id,name,modifiedTime",
                            createMeta,
                            "application/octet-stream",
                            figBytes
                    );
                }

                String finalId = uploaded.optString("id", existingId);
                putMappedFileId(docUid, finalId);
                upsertIndexEntry(
                        finalId,
                        figFileName,
                        uploaded.optString("modifiedTime", ""),
                        cachedFig.getAbsolutePath(),
                        cachedThumbPath,
                        true
                );
                listener.onStatus("Drive 동기화 완료: " + figFileName);
            } catch (Exception e) {
                listener.onStatus("Drive 업로드 오류: " + e.getMessage());
            }
        }, "figcraft-drive-upload").start();
    }

    void deleteDriveFileAsync(String fileId, String name) {
        new Thread(() -> {
            try {
                if (fileId != null && !fileId.isEmpty() && ensureAccessToken()) {
                    HttpURLConnection conn = (HttpURLConnection) new URL("https://www.googleapis.com/drive/v3/files/" + enc(fileId)).openConnection();
                    conn.setRequestMethod("DELETE");
                    conn.setConnectTimeout(15_000);
                    conn.setReadTimeout(20_000);
                    conn.setRequestProperty("Authorization", "Bearer " + accessToken);
                    conn.getResponseCode();
                    conn.disconnect();
                }
                removeMappedFileId(fileId);
                String safeName = sanitizeCanvasName(name);
                if (!safeName.endsWith(".figcraft")) safeName += ".figcraft";
                File cached = new File(cacheDir, safeName);
                if (cached.exists()) {
                    //noinspection ResultOfMethodCallIgnored
                    cached.delete();
                }
                removeIndexEntry(fileId, name);
                listener.onStatus("Drive 파일 삭제 완료: " + safeName);
            } catch (Exception e) {
                listener.onStatus("Drive 삭제 오류: " + e.getMessage());
            }
        }, "figcraft-drive-delete").start();
    }

    void openDriveFileAsync(String fileId, String name) {
        new Thread(() -> {
            try {
                String safeName = sanitizeCanvasName(name);
                if (!safeName.endsWith(".figcraft")) safeName += ".figcraft";
                File cached = new File(cacheDir, safeName);
                if (fileId != null && !fileId.isEmpty() && ensureAccessToken()) {
                    byte[] bytes = httpGetBytes("https://www.googleapis.com/drive/v3/files/" + enc(fileId) + "?alt=media", accessToken);
                    if (bytes.length > 0) {
                        writeBytes(cached, bytes);
                    }
                }
                if (cached.exists()) {
                    listener.onFileReadyToOpen(cached.getAbsolutePath(), safeName);
                } else {
                    listener.onStatus("Drive 파일을 찾을 수 없습니다: " + safeName);
                }
            } catch (Exception e) {
                listener.onStatus("Drive 열기 오류: " + e.getMessage());
            }
        }, "figcraft-drive-open").start();
    }

    private void syncAndListBlocking() {
        try {
            if (!ensureAccessToken()) {
                listener.onStatus("Google 로그인이 필요합니다.");
                return;
            }
            String fid = ensureFolderId();
            if (fid.isEmpty()) return;

            String q = "'" + fid + "' in parents and trashed = false";
            String url = "https://www.googleapis.com/drive/v3/files?q=" + enc(q)
                    + "&orderBy=modifiedTime desc&pageSize=50&fields=files(id,name,modifiedTime,mimeType)";
            JSONObject res = httpJson("GET", url, accessToken, null, null);
            JSONArray files = res.optJSONArray("files");
            JSONArray outItems = new JSONArray();
            if (files != null) {
                for (int i = 0; i < files.length(); i++) {
                    JSONObject f = files.optJSONObject(i);
                    if (f == null) continue;
                    String name = f.optString("name", "");
                    if (!name.endsWith(".figcraft")) continue;
                    String id = f.optString("id", "");
                    String mod = f.optString("modifiedTime", "");
                    File cached = new File(cacheDir, sanitizeCanvasName(name));
                    File thumb = new File(cacheDir, sanitizeCanvasName(name) + ".thumb.png");
                    if (!cached.exists()) {
                        try {
                            byte[] data = httpGetBytes("https://www.googleapis.com/drive/v3/files/" + enc(id) + "?alt=media", accessToken);
                            if (data.length > 0) writeBytes(cached, data);
                        } catch (Exception ignored) {
                        }
                    }
                    JSONObject item = new JSONObject();
                    item.put("file_id", id);
                    item.put("name", name);
                    item.put("modified_time", mod);
                    item.put("local_path", cached.exists() ? cached.getAbsolutePath() : "");
                    item.put("thumb_path", thumb.exists() ? thumb.getAbsolutePath() : "");
                    item.put("synced", true);
                    outItems.put(item);
                }
            }
            JSONObject idx = new JSONObject();
            idx.put("items", outItems);
            String json = idx.toString();
            writeBytes(indexFile, json.getBytes(StandardCharsets.UTF_8));
            listener.onIndexUpdated(json);
            listener.onStatus("Drive 폴더 동기화 완료 (" + outItems.length() + "개)");
        } catch (Exception e) {
            listener.onStatus("Drive 목록 조회 오류: " + e.getMessage());
        }
    }

    private synchronized boolean ensureAccessToken() throws Exception {
        if (!accessToken.isEmpty() && System.currentTimeMillis() < expiresAtMs) {
            return true;
        }
        if (refreshToken.isEmpty() || clientId.isEmpty()) {
            return false;
        }
        String body = "client_id=" + enc(clientId)
                + (clientSecret.isEmpty() ? "" : ("&client_secret=" + enc(clientSecret)))
                + "&refresh_token=" + enc(refreshToken)
                + "&grant_type=refresh_token";
        JSONObject tok = httpJson("POST", "https://oauth2.googleapis.com/token", null, "application/x-www-form-urlencoded", body.getBytes(StandardCharsets.UTF_8));
        accessToken = tok.optString("access_token", "");
        long expiresIn = tok.optLong("expires_in", 3600L);
        expiresAtMs = System.currentTimeMillis() + Math.max(60L, expiresIn - 60L) * 1000L;
        saveAuth();
        return !accessToken.isEmpty();
    }

    private String ensureFolderId() throws Exception {
        if (!folderId.isEmpty()) return folderId;
        String q = "name = '" + FOLDER_NAME + "' and mimeType = 'application/vnd.google-apps.folder' and trashed = false";
        JSONObject res = httpJson("GET", "https://www.googleapis.com/drive/v3/files?q=" + enc(q) + "&fields=files(id,name)", accessToken, null, null);
        JSONArray files = res.optJSONArray("files");
        if (files != null && files.length() > 0) {
            folderId = files.getJSONObject(0).optString("id", "");
            if (!folderId.isEmpty()) return folderId;
        }
        JSONObject meta = new JSONObject();
        meta.put("name", FOLDER_NAME);
        meta.put("mimeType", "application/vnd.google-apps.folder");
        JSONObject created = httpJson("POST", "https://www.googleapis.com/drive/v3/files?fields=id", accessToken, "application/json; charset=UTF-8", meta.toString().getBytes(StandardCharsets.UTF_8));
        folderId = created.optString("id", "");
        return folderId;
    }

    private String findFileInFolder(String fid, String name) throws Exception {
        String escaped = name.replace("'", "\\'");
        String q = "'" + fid + "' in parents and name = '" + escaped + "' and trashed = false";
        JSONObject res = httpJson("GET", "https://www.googleapis.com/drive/v3/files?q=" + enc(q) + "&fields=files(id)", accessToken, null, null);
        JSONArray files = res.optJSONArray("files");
        if (files != null && files.length() > 0) {
            return files.getJSONObject(0).optString("id", "");
        }
        return "";
    }

    private String fetchEmail(String token) {
        try {
            JSONObject info = httpJson("GET", "https://www.googleapis.com/oauth2/v2/userinfo", token, null, null);
            return info.optString("email", "");
        } catch (Exception ignored) {
            return "";
        }
    }

    private synchronized void upsertIndexEntry(String fileId, String name, String modTime, String localPath, String thumbPath, boolean synced) {
        try {
            JSONObject idx = indexFile.exists() ? new JSONObject(readText(indexFile)) : new JSONObject();
            JSONArray oldArr = idx.optJSONArray("items");
            JSONArray nextArr = new JSONArray();
            JSONObject entry = new JSONObject();
            entry.put("file_id", fileId);
            entry.put("name", name);
            entry.put("modified_time", modTime);
            entry.put("local_path", localPath);
            entry.put("thumb_path", thumbPath);
            entry.put("synced", synced);
            nextArr.put(entry);
            if (oldArr != null) {
                for (int i = 0; i < oldArr.length(); i++) {
                    JSONObject o = oldArr.optJSONObject(i);
                    if (o == null) continue;
                    if (!name.equals(o.optString("name", "")) && (fileId.isEmpty() || !fileId.equals(o.optString("file_id", "")))) {
                        nextArr.put(o);
                    }
                }
            }
            JSONObject out = new JSONObject();
            out.put("items", nextArr);
            String json = out.toString();
            writeBytes(indexFile, json.getBytes(StandardCharsets.UTF_8));
            listener.onIndexUpdated(json);
        } catch (Exception ignored) {
        }
    }

    private synchronized void removeIndexEntry(String fileId, String name) {
        try {
            if (!indexFile.exists()) return;
            JSONObject idx = new JSONObject(readText(indexFile));
            JSONArray oldArr = idx.optJSONArray("items");
            JSONArray nextArr = new JSONArray();
            if (oldArr != null) {
                for (int i = 0; i < oldArr.length(); i++) {
                    JSONObject o = oldArr.optJSONObject(i);
                    if (o == null) continue;
                    boolean sameId = fileId != null && !fileId.isEmpty() && fileId.equals(o.optString("file_id", ""));
                    boolean sameName = name != null && !name.isEmpty() && name.equals(o.optString("name", ""));
                    if (!sameId && !sameName) {
                        nextArr.put(o);
                    }
                }
            }
            JSONObject out = new JSONObject();
            out.put("items", nextArr);
            String json = out.toString();
            writeBytes(indexFile, json.getBytes(StandardCharsets.UTF_8));
            listener.onIndexUpdated(json);
        } catch (Exception ignored) {
        }
    }

    private JSONObject uploadMultipart(String method, String urlStr, JSONObject meta, String mime, byte[] data) throws Exception {
        String boundary = "figcraft_" + UUID.randomUUID().toString().replace("-", "");
        ByteArrayOutputStream bos = new ByteArrayOutputStream();
        String part1 = "--" + boundary + "\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n" + meta.toString() + "\r\n";
        String part2Head = "--" + boundary + "\r\nContent-Type: " + mime + "\r\n\r\n";
        String tail = "\r\n--" + boundary + "--\r\n";
        bos.write(part1.getBytes(StandardCharsets.UTF_8));
        bos.write(part2Head.getBytes(StandardCharsets.UTF_8));
        bos.write(data);
        bos.write(tail.getBytes(StandardCharsets.UTF_8));
        return httpJson(method, urlStr, accessToken, "multipart/related; boundary=" + boundary, bos.toByteArray());
    }

    private JSONObject httpJson(String method, String urlStr, String bearer, String contentType, byte[] body) throws Exception {
        HttpURLConnection conn = (HttpURLConnection) new URL(urlStr).openConnection();
        conn.setRequestMethod(method);
        conn.setConnectTimeout(15_000);
        conn.setReadTimeout(30_000);
        if (bearer != null && !bearer.isEmpty()) {
            conn.setRequestProperty("Authorization", "Bearer " + bearer);
        }
        if (contentType != null) {
            conn.setRequestProperty("Content-Type", contentType);
        }
        if (body != null) {
            conn.setDoOutput(true);
            try (OutputStream os = conn.getOutputStream()) {
                os.write(body);
            }
        }
        int code = conn.getResponseCode();
        InputStream is = code >= 400 ? conn.getErrorStream() : conn.getInputStream();
        String text = is != null ? new String(readStream(is), StandardCharsets.UTF_8) : "{}";
        if (code >= 400) {
            throw new IllegalStateException("HTTP " + code + ": " + text);
        }
        return new JSONObject(text);
    }

    private byte[] httpGetBytes(String urlStr, String bearer) throws Exception {
        HttpURLConnection conn = (HttpURLConnection) new URL(urlStr).openConnection();
        conn.setRequestMethod("GET");
        conn.setConnectTimeout(15_000);
        conn.setReadTimeout(30_000);
        if (bearer != null && !bearer.isEmpty()) {
            conn.setRequestProperty("Authorization", "Bearer " + bearer);
        }
        int code = conn.getResponseCode();
        if (code >= 400) {
            throw new IllegalStateException("HTTP " + code);
        }
        try (InputStream is = conn.getInputStream()) {
            return readStream(is);
        }
    }

    private synchronized void loadAuth() {
        if (!authFile.exists()) return;
        try {
            JSONObject obj = new JSONObject(readText(authFile));
            clientId = obj.optString("client_id", "");
            clientSecret = obj.optString("client_secret", "");
            accessToken = obj.optString("access_token", "");
            refreshToken = obj.optString("refresh_token", "");
            expiresAtMs = obj.optLong("expires_at_ms", 0L);
            userEmail = obj.optString("user_email", "");
            folderId = obj.optString("folder_id", "");
        } catch (Exception ignored) {
        }
    }

    private synchronized void saveAuth() {
        try {
            JSONObject obj = new JSONObject();
            obj.put("client_id", clientId);
            obj.put("client_secret", clientSecret);
            obj.put("access_token", accessToken);
            obj.put("refresh_token", refreshToken);
            obj.put("expires_at_ms", expiresAtMs);
            obj.put("user_email", userEmail);
            obj.put("folder_id", folderId);
            writeBytes(authFile, obj.toString().getBytes(StandardCharsets.UTF_8));
        } catch (Exception ignored) {
        }
    }

    private static String extractQueryParam(String httpLine, String key) {
        if (httpLine == null) return null;
        int q = httpLine.indexOf('?');
        int sp = httpLine.indexOf(' ', Math.max(0, q));
        if (q < 0 || sp < 0) return null;
        String query = httpLine.substring(q + 1, sp);
        for (String pair : query.split("&")) {
            int eq = pair.indexOf('=');
            if (eq > 0 && key.equals(pair.substring(0, eq))) {
                try {
                    return java.net.URLDecoder.decode(pair.substring(eq + 1), "UTF-8");
                } catch (Exception ignored) {
                }
            }
        }
        return null;
    }

    private static String sanitizeCanvasName(String name) {
        String base = (name == null || name.trim().isEmpty()) ? "Untitled.figcraft" : name.trim();
        return base.replaceAll("[\\\\/:*?\"<>|]", "_");
    }

    private static String enc(String s) {
        try {
            return URLEncoder.encode(s, "UTF-8");
        } catch (Exception e) {
            return s;
        }
    }

    private static String readText(File f) throws Exception {
        return new String(readBytes(f), StandardCharsets.UTF_8);
    }

    private static byte[] readBytes(File f) throws Exception {
        try (FileInputStream fis = new FileInputStream(f)) {
            return readStream(fis);
        }
    }

    private static byte[] readStream(InputStream is) throws Exception {
        ByteArrayOutputStream bos = new ByteArrayOutputStream();
        byte[] buf = new byte[16 * 1024];
        int n;
        while ((n = is.read(buf)) > 0) {
            bos.write(buf, 0, n);
        }
        return bos.toByteArray();
    }

    private static void writeBytes(File f, byte[] data) throws Exception {
        try (FileOutputStream fos = new FileOutputStream(f)) {
            fos.write(data);
        }
    }
}
