// Ferrite JDBC 사이드카. Rust에 드라이버가 없는 DB(Oracle·Tibero·CUBRID)를 JDBC로 연다.
//
// 표준 입력으로 한 줄에 JSON 요청 하나({"method": ..., "params": {...}})를 받아 표준 출력에 한 줄 응답
// ({"result": ...} 또는 {"error": "..."})을 쓴다. "cancel"만 응답 없이, 읽는 스레드에서 바로 실행 중인
// 문장을 멈춘다. 값은 PreparedStatement 파라미터로만 넣는다(SQL 글에 붙이지 않는다).
//
// Java 8에서도 돌도록 쓴다(Ferrite가 사용자 JDK의 javac로 처음 쓸 때 컴파일한다).

import java.io.BufferedReader;
import java.io.FileDescriptor;
import java.io.FileOutputStream;
import java.io.InputStreamReader;
import java.io.PrintStream;
import java.nio.charset.StandardCharsets;
import java.sql.Blob;
import java.sql.Connection;
import java.sql.DatabaseMetaData;
import java.sql.Driver;
import java.sql.PreparedStatement;
import java.sql.ResultSet;
import java.sql.ResultSetMetaData;
import java.sql.SQLException;
import java.sql.Statement;
import java.sql.Types;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Properties;
import java.util.concurrent.BlockingQueue;
import java.util.concurrent.LinkedBlockingQueue;

public final class FerriteJdbc {
    private final int pageRows;
    private Connection conn;
    /** 다시 연결할 때 쓴다(세션을 끊어 멈춘 뒤). */
    private Driver driver;
    private String url;
    private Properties properties;
    private boolean autoCommit = true;
    /** 세션을 끊어 멈출 수 있으면 이 연결의 (SID, SERIAL#). Tibero만. */
    private volatile String[] session;
    /** 세션을 끊은 연결. 지금 연결이 이것이면 실패한 문장 뒤에 다시 연결한다. */
    private volatile Connection killed;
    /** 실행 중인 문장(`running`)을 끝내는 일과 세션 끊기를 묶는다. 끊는 중이면 `killing`. */
    private final Object killLock = new Object();
    private boolean killing;
    private Statement statement;
    private ResultSet result;
    /** 실행·읽기 중인 문장. 읽는 스레드가 멈출 때 쓴다. */
    private volatile Statement running;

    private FerriteJdbc(int pageRows) {
        this.pageRows = pageRows;
    }

    public static void main(String[] args) throws Exception {
        final PrintStream out = new PrintStream(new FileOutputStream(FileDescriptor.out), true, "UTF-8");
        // 드라이버가 표준 출력에 쓰는 기록이 응답과 섞이지 않게 한다.
        System.setOut(System.err);
        final FerriteJdbc sidecar = new FerriteJdbc(Integer.parseInt(args[0]));
        final BlockingQueue<Object> queue = new LinkedBlockingQueue<Object>();
        final Object end = new Object();
        Thread reader = new Thread(new Runnable() {
            public void run() {
                try {
                    BufferedReader in = new BufferedReader(new InputStreamReader(System.in, StandardCharsets.UTF_8));
                    String line;
                    while ((line = in.readLine()) != null) {
                        if (line.trim().isEmpty()) {
                            continue;
                        }
                        Map<String, Object> request = Json.object(line);
                        if ("cancel".equals(request.get("method"))) {
                            sidecar.cancel();
                        } else {
                            queue.put(request);
                        }
                    }
                } catch (Exception e) {
                    e.printStackTrace();
                }
                // 입력이 끝났다(Ferrite가 연결을 닫았거나 끝났다). 실행 중인 문장을 멈추고, 그래도
                // 끝나지 않으면(멈춤이 듣지 않는 망) 프로세스를 끝낸다. 서버는 끊긴 연결을 롤백한다.
                sidecar.cancel();
                Thread watchdog = new Thread(new Runnable() {
                    public void run() {
                        try {
                            Thread.sleep(10000);
                        } catch (InterruptedException ignored) {
                            Thread.currentThread().interrupt();
                        }
                        Runtime.getRuntime().halt(1);
                    }
                }, "ferrite-jdbc-watchdog");
                watchdog.setDaemon(true);
                watchdog.start();
                try {
                    queue.put(end);
                } catch (InterruptedException ignored) {
                    Thread.currentThread().interrupt();
                }
            }
        }, "ferrite-jdbc-reader");
        reader.setDaemon(true);
        reader.start();
        while (true) {
            Object next = queue.take();
            if (next == end) {
                break;
            }
            @SuppressWarnings("unchecked")
            Map<String, Object> request = (Map<String, Object>) next;
            Map<String, Object> response = new LinkedHashMap<String, Object>();
            try {
                response.put("result", sidecar.handle((String) request.get("method"), params(request)));
            } catch (Throwable e) {
                response.put("error", message(e));
            }
            out.println(Json.write(response));
        }
        sidecar.shutdown();
    }

    @SuppressWarnings("unchecked")
    private static Map<String, Object> params(Map<String, Object> request) {
        Object params = request.get("params");
        return params instanceof Map ? (Map<String, Object>) params : new LinkedHashMap<String, Object>();
    }

    private static String message(Throwable e) {
        String text = e.getMessage();
        if (text == null || text.trim().isEmpty()) {
            text = e.getClass().getName();
        }
        return text.trim();
    }

    private Object handle(String method, Map<String, Object> params) throws Exception {
        if ("open".equals(method)) {
            return open(params);
        }
        if (conn == null) {
            throw new IllegalStateException("연결하지 않았습니다");
        }
        Map<String, Object> done = new LinkedHashMap<String, Object>();
        if ("execute".equals(method)) {
            return execute((String) params.get("sql"));
        } else if ("fetch".equals(method)) {
            return page();
        } else if ("close".equals(method)) {
            closeResult();
        } else if ("schema".equals(method)) {
            closeResult();
            return schema((String) params.get("synonyms"));
        } else if ("table".equals(method)) {
            // 결과를 읽는 중에도 묻는다(결과를 닫지 않는다).
            return table(params);
        } else if ("update".equals(method)) {
            done.put("affected", update((String) params.get("sql"), (List<?>) params.get("params")));
        } else if ("commit".equals(method)) {
            closeResult();
            if (!conn.getAutoCommit()) {
                conn.commit();
            }
        } else if ("rollback".equals(method)) {
            closeResult();
            if (!conn.getAutoCommit()) {
                conn.rollback();
            }
        } else if ("autocommit".equals(method)) {
            closeResult();
            // JDBC 규약대로 켜면 열린 트랜잭션을 커밋한다.
            autoCommit = Boolean.TRUE.equals(params.get("on"));
            conn.setAutoCommit(autoCommit);
        } else {
            throw new IllegalArgumentException("모르는 요청: " + method);
        }
        return done;
    }

    private Map<String, Object> open(Map<String, Object> params) throws Exception {
        String driverClass = (String) params.get("driver");
        try {
            driver = (Driver) Class.forName(driverClass).getDeclaredConstructor().newInstance();
        } catch (ClassNotFoundException e) {
            throw new IllegalStateException("드라이버 클래스를 찾지 못했습니다(" + driverClass + "). 드라이버 jar를 확인하세요");
        }
        properties = new Properties();
        Object extra = params.get("properties");
        if (extra instanceof Map) {
            for (Map.Entry<?, ?> entry : ((Map<?, ?>) extra).entrySet()) {
                properties.setProperty(entry.getKey().toString(), String.valueOf(entry.getValue()));
            }
        }
        properties.setProperty("user", (String) params.get("user"));
        Object password = params.get("password");
        if (password != null) {
            properties.setProperty("password", (String) password);
        }
        url = (String) params.get("url");
        conn = driver.connect(url, properties);
        if (conn == null) {
            throw new IllegalStateException("드라이버가 이 URL을 받지 않습니다: " + url);
        }
        if (Boolean.TRUE.equals(params.get("stopByKill"))) {
            session = ownSession();
        }
        DatabaseMetaData meta = conn.getMetaData();
        String name = meta.getDatabaseProductName();
        String version = meta.getDatabaseProductVersion();
        Map<String, Object> opened = new LinkedHashMap<String, Object>();
        // Oracle은 버전에 제품 이름이 들어 있다.
        opened.put("version", version.contains(name) ? version : name + " " + version);
        return opened;
    }

    private Map<String, Object> execute(String sql) throws SQLException {
        closeResult();
        Statement st = conn.createStatement();
        Map<String, Object> done = new LinkedHashMap<String, Object>();
        try {
            st.setFetchSize(pageRows);
            running = st;
            if (st.execute(sql)) {
                statement = st;
                result = st.getResultSet();
                ResultSetMetaData meta = result.getMetaData();
                List<Object> columns = new ArrayList<Object>();
                for (int i = 1; i <= meta.getColumnCount(); i++) {
                    columns.add(meta.getColumnLabel(i));
                }
                Map<String, Object> page = page();
                done.put("columns", columns);
                done.putAll(page);
                return done;
            }
            int count = st.getUpdateCount();
            done.put("affected", count >= 0 ? Long.valueOf(count) : null);
            st.close();
            return done;
        } catch (SQLException e) {
            closeQuietly(st);
            statement = null;
            result = null;
            throw afterKill(e);
        } finally {
            finished();
        }
    }

    /** 다음 페이지. 끝까지 읽었으면 결과를 닫는다. */
    private Map<String, Object> page() throws SQLException {
        Map<String, Object> page = new LinkedHashMap<String, Object>();
        List<Object> rows = new ArrayList<Object>();
        boolean more = false;
        if (result != null) {
            running = statement;
            try {
                ResultSetMetaData meta = result.getMetaData();
                int count = meta.getColumnCount();
                while (rows.size() < pageRows && result.next()) {
                    List<Object> row = new ArrayList<Object>(count);
                    for (int i = 1; i <= count; i++) {
                        row.add(cell(i, meta.getColumnType(i)));
                    }
                    rows.add(row);
                }
                // 페이지를 다 채웠으면 남은 행이 있다고 본다(끝이면 다음 fetch가 빈 페이지다).
                more = rows.size() == pageRows;
            } catch (SQLException e) {
                closeResult();
                throw afterKill(e);
            } finally {
                finished();
            }
            if (!more) {
                closeResult();
            }
        }
        page.put("rows", rows);
        page.put("more", more);
        return page;
    }

    /** 칸 하나. NULL은 null, 이진 값은 {"hex": ...}, 나머지는 드라이버가 보이는 글자 그대로. */
    private Object cell(int i, int type) throws SQLException {
        byte[] bytes = null;
        switch (type) {
            case Types.BINARY:
            case Types.VARBINARY:
            case Types.LONGVARBINARY:
                bytes = result.getBytes(i);
                break;
            case Types.BLOB:
                Blob blob = result.getBlob(i);
                if (blob != null) {
                    bytes = blob.getBytes(1, (int) blob.length());
                    blob.free();
                }
                break;
            default:
                String text;
                try {
                    text = result.getString(i);
                } catch (SQLException e) {
                    // 글자로 못 읽는 형식(구조체 등)은 객체를 글자로 보인다.
                    Object value = result.getObject(i);
                    text = value == null ? null : value.toString();
                }
                return text;
        }
        if (bytes == null) {
            return null;
        }
        StringBuilder hex = new StringBuilder(bytes.length * 2);
        for (byte b : bytes) {
            hex.append(Character.forDigit((b >> 4) & 0xf, 16)).append(Character.forDigit(b & 0xf, 16));
        }
        Map<String, Object> value = new LinkedHashMap<String, Object>();
        value.put("hex", hex.toString());
        return value;
    }

    /** 식별자를 DB가 두는 대소문자로. */
    private static String fold(String text, boolean upper, boolean lower) {
        if (upper) {
            return text.toUpperCase(Locale.ROOT);
        }
        return lower ? text.toLowerCase(Locale.ROOT) : text;
    }

    private static String mode(boolean upper, boolean lower) {
        return upper ? "upper" : lower ? "lower" : "none";
    }

    /** {"text", "quoted"} 식별자를 DB가 두는 대소문자로. */
    private static String identifier(Object value, DatabaseMetaData meta) throws SQLException {
        Map<?, ?> ident = (Map<?, ?>) value;
        String text = (String) ident.get("text");
        if (Boolean.TRUE.equals(ident.get("quoted"))) {
            return fold(text, meta.storesUpperCaseQuotedIdentifiers(), meta.storesLowerCaseQuotedIdentifiers());
        }
        return fold(text, meta.storesUpperCaseIdentifiers(), meta.storesLowerCaseIdentifiers());
    }

    /**
     * 표의 열(순서대로, 형식 이름과 함께)과 기본 키. 스키마를 적지 않았으면 사용자 스키마의 표만 찾는다
     * (다른 스키마의 같은 이름 표나 동의어가 가리키는 표를 고치지 않게). 없거나 둘 이상이면 null.
     * CUBRID는 스키마 칸이 비고 표 이름이 "소유자.표"로 온다.
     */
    private Map<String, Object> table(Map<String, Object> params) throws SQLException {
        DatabaseMetaData meta = conn.getMetaData();
        String user = meta.getUserName();
        String schema = params.get("schema") != null
            ? identifier(params.get("schema"), meta)
            : fold(user, meta.storesUpperCaseIdentifiers(), meta.storesLowerCaseIdentifiers());
        String table = identifier(params.get("table"), meta);
        List<Object> columns = new ArrayList<Object>();
        String foundSchema = null;
        ResultSet rs = meta.getColumns(null, schema, table, null);
        try {
            while (rs.next()) {
                String[] name = qualified(rs.getString("TABLE_SCHEM"), rs.getString("TABLE_NAME"));
                if (!table.equals(name[1]) || (name[0] != null && !schema.equals(name[0]))) {
                    continue;
                }
                if (!columns.isEmpty() && !java.util.Objects.equals(foundSchema, name[0])) {
                    return null;
                }
                foundSchema = name[0];
                Map<String, Object> column = new LinkedHashMap<String, Object>();
                column.put("name", rs.getString("COLUMN_NAME"));
                column.put("type", rs.getString("TYPE_NAME"));
                columns.add(column);
            }
        } finally {
            rs.close();
        }
        if (columns.isEmpty()) {
            return null;
        }
        Map<Integer, String> key = new java.util.TreeMap<Integer, String>();
        rs = meta.getPrimaryKeys(null, schema, table);
        try {
            while (rs.next()) {
                String[] name = qualified(rs.getString("TABLE_SCHEM"), rs.getString("TABLE_NAME"));
                if (table.equals(name[1]) && (name[0] == null || schema.equals(name[0]))) {
                    key.put(Integer.valueOf(rs.getInt("KEY_SEQ")), rs.getString("COLUMN_NAME"));
                }
            }
        } finally {
            rs.close();
        }
        Map<String, Object> info = new LinkedHashMap<String, Object>();
        info.put("schema", foundSchema);
        info.put("table", table);
        info.put("columns", columns);
        info.put("key", new ArrayList<Object>(key.values()));
        info.put("fold", mode(meta.storesUpperCaseIdentifiers(), meta.storesLowerCaseIdentifiers()));
        info.put("quoted", mode(meta.storesUpperCaseQuotedIdentifiers(), meta.storesLowerCaseQuotedIdentifiers()));
        return info;
    }

    /**
     * 사용자 스키마의 표·뷰와 열(스키마 브라우저). 열은 스키마 전체를 한 번에 읽는다(표마다 묻지 않는다).
     * Oracle 휴지통의 표(BIN$…)는 뺀다. {@code synonyms}가 있으면 그 조회(동의어, 표 주인, 표, 열, 형식)로
     * 동의어도 더한다. 같은 이름의 표나 앞서 읽은 동의어(사용자 동의어가 PUBLIC보다 먼저 온다)가 있으면
     * 그것이 이긴다(DB가 이름을 찾는 순서와 같다).
     */
    private Map<String, Object> schema(String synonyms) throws SQLException {
        DatabaseMetaData meta = conn.getMetaData();
        String user = fold(meta.getUserName(), meta.storesUpperCaseIdentifiers(), meta.storesLowerCaseIdentifiers());
        Map<String, Map<String, Object>> tables = new java.util.TreeMap<String, Map<String, Object>>();
        ResultSet rs = meta.getTables(null, user, "%", new String[] {"TABLE", "VIEW"});
        try {
            while (rs.next()) {
                String[] name = qualified(rs.getString("TABLE_SCHEM"), rs.getString("TABLE_NAME"));
                if ((name[0] != null && !user.equals(name[0])) || name[1].startsWith("BIN$")) {
                    continue;
                }
                Map<String, Object> table = new LinkedHashMap<String, Object>();
                table.put("name", name[1]);
                table.put("view", "VIEW".equalsIgnoreCase(rs.getString("TABLE_TYPE")));
                table.put("columns", new ArrayList<Object>());
                tables.put(name[1], table);
            }
        } finally {
            rs.close();
        }
        rs = meta.getColumns(null, user, "%", "%");
        try {
            while (rs.next()) {
                String[] name = qualified(rs.getString("TABLE_SCHEM"), rs.getString("TABLE_NAME"));
                Map<String, Object> table = tables.get(name[1]);
                if (table == null || (name[0] != null && !user.equals(name[0]))) {
                    continue;
                }
                Map<String, Object> column = new LinkedHashMap<String, Object>();
                column.put("name", rs.getString("COLUMN_NAME"));
                column.put("type", rs.getString("TYPE_NAME"));
                @SuppressWarnings("unchecked")
                List<Object> columns = (List<Object>) table.get("columns");
                columns.add(column);
            }
        } finally {
            rs.close();
        }
        if (synonyms != null) {
            addSynonyms(synonyms, tables);
        }
        Map<String, Object> result = new LinkedHashMap<String, Object>();
        result.put("tables", new ArrayList<Object>(tables.values()));
        return result;
    }

    /** 동의어 조회의 줄을 동의어마다 묶어 더한다. 사전을 못 읽으면(권한 등) 동의어 없이 둔다. */
    private void addSynonyms(String sql, Map<String, Map<String, Object>> tables) {
        Statement st = null;
        try {
            st = conn.createStatement();
            ResultSet rs = st.executeQuery(sql);
            Map<String, Object> current = null;
            String currentName = null;
            boolean taken = false;
            while (rs.next()) {
                String name = rs.getString(1);
                if (!name.equals(currentName)) {
                    currentName = name;
                    taken = !tables.containsKey(name);
                    if (taken) {
                        current = new LinkedHashMap<String, Object>();
                        current.put("name", name);
                        current.put("view", Boolean.FALSE);
                        current.put("synonym", rs.getString(2) + "." + rs.getString(3));
                        current.put("columns", new ArrayList<Object>());
                        tables.put(name, current);
                    }
                }
                if (!taken) {
                    continue;
                }
                Map<String, Object> column = new LinkedHashMap<String, Object>();
                column.put("name", rs.getString(4));
                column.put("type", rs.getString(5));
                @SuppressWarnings("unchecked")
                List<Object> columns = (List<Object>) current.get("columns");
                columns.add(column);
            }
            rs.close();
        } catch (SQLException e) {
            e.printStackTrace();
        } finally {
            closeQuietly(st);
        }
    }

    /** (스키마, 표). 스키마 칸이 비고 표 이름에 점이 있으면 나눈다(CUBRID 11.2). */
    private static String[] qualified(String schema, String table) {
        if (schema == null && table != null && table.indexOf('.') > 0) {
            return table.split("\\.", 2);
        }
        return new String[] {schema, table};
    }

    private long update(String sql, List<?> params) throws SQLException {
        closeResult();
        PreparedStatement st = conn.prepareStatement(sql);
        try {
            for (int i = 0; i < params.size(); i++) {
                Object value = params.get(i);
                if (value == null) {
                    st.setNull(i + 1, Types.VARCHAR);
                } else if (value instanceof Map) {
                    st.setBytes(i + 1, hex((String) ((Map<?, ?>) value).get("hex")));
                } else {
                    st.setString(i + 1, value.toString());
                }
            }
            running = st;
            return st.executeUpdate();
        } finally {
            finished();
            closeQuietly(st);
        }
    }

    private static byte[] hex(String text) {
        byte[] bytes = new byte[text.length() / 2];
        for (int i = 0; i < bytes.length; i++) {
            bytes[i] = (byte) Integer.parseInt(text.substring(i * 2, i * 2 + 2), 16);
        }
        return bytes;
    }

    private void cancel() {
        final Statement st = running;
        if (st == null) {
            return;
        }
        try {
            st.cancel();
        } catch (SQLException e) {
            e.printStackTrace();
        }
        if (session == null) {
            return;
        }
        // Tibero 6은 멈춤 요청을 받고도 문장을 1분 가까이 더 돌린다. 3초 안에 멈추지 않으면 다른
        // 연결에서 이 세션을 끊는다(ALTER SYSTEM 권한이 있을 때만 된다).
        Thread killer = new Thread(new Runnable() {
            public void run() {
                try {
                    Thread.sleep(3000);
                    String[] own = session;
                    Connection target = conn;
                    synchronized (killLock) {
                        // 그 사이 문장이 끝났으면 끊지 않는다(쉬는 연결을 끊게 된다).
                        if (running != st || own == null) {
                            return;
                        }
                        killing = true;
                    }
                    try {
                        Connection helper = driver.connect(url, properties);
                        try {
                            Statement kill = helper.createStatement();
                            killed = target;
                            kill.execute("ALTER SYSTEM KILL SESSION (" + own[0] + ", " + own[1] + ")");
                            kill.close();
                        } finally {
                            helper.close();
                        }
                    } finally {
                        synchronized (killLock) {
                            killing = false;
                            killLock.notifyAll();
                        }
                    }
                } catch (Exception e) {
                    e.printStackTrace();
                }
            }
        }, "ferrite-jdbc-killer");
        killer.setDaemon(true);
        killer.start();
    }

    /** 이 연결의 (SID, SERIAL#). 볼 권한이 없으면 null(세션을 끊어 멈추지 않는다). */
    private String[] ownSession() {
        try {
            Statement st = conn.createStatement();
            try {
                ResultSet rs = st.executeQuery(
                    "SELECT SID, SERIAL# FROM V$SESSION WHERE SID = (SELECT SID FROM V$MYSTAT WHERE ROWNUM = 1)");
                if (!rs.next()) {
                    return null;
                }
                String sid = rs.getString(1);
                String serial = rs.getString(2);
                // 숫자만 SQL에 넣는다.
                return sid.matches("\\d+") && serial.matches("\\d+") ? new String[] {sid, serial} : null;
            } finally {
                st.close();
            }
        } catch (SQLException e) {
            return null;
        }
    }

    /** 세션을 끊어 멈췄으면 다시 연결하고(자동 커밋 상태는 되살린다) 그 사실을 알리는 오류로 바꾼다. */
    /** 문장이 끝났다. 세션을 끊는 중이면 끝날 때까지 기다린다(그 뒤 다음 일이 죽은 연결을 쓰지 않게). */
    private void finished() {
        synchronized (killLock) {
            running = null;
            while (killing) {
                try {
                    killLock.wait();
                } catch (InterruptedException e) {
                    Thread.currentThread().interrupt();
                    return;
                }
            }
        }
    }

    private SQLException afterKill(SQLException e) {
        finished();
        if (killed == null || killed != conn) {
            return e;
        }
        killed = null;
        try {
            conn = driver.connect(url, properties);
            conn.setAutoCommit(autoCommit);
            session = ownSession();
            return new SQLException("문장을 멈추려고 세션을 끊고 다시 연결했습니다(커밋하지 않은 변경은 롤백됐습니다)", e);
        } catch (SQLException again) {
            return new SQLException("문장을 멈추려고 세션을 끊었는데 다시 연결하지 못했습니다: " + message(again), e);
        }
    }

    private void closeResult() {
        if (result != null) {
            try {
                result.close();
            } catch (SQLException ignored) {
                // 닫다가 난 오류는 다음 문장에 영향이 없다.
            }
            result = null;
        }
        closeQuietly(statement);
        statement = null;
    }

    private static void closeQuietly(Statement st) {
        if (st != null) {
            try {
                st.close();
            } catch (SQLException ignored) {
                // 위와 같다.
            }
        }
    }

    /** 입력이 끝나면(Ferrite가 연결을 닫으면) 끝낸다. Oracle은 닫을 때 열린 트랜잭션을 커밋하므로
     *  먼저 롤백한다(다른 DB처럼 커밋하지 않은 변경은 버린다). */
    private void shutdown() {
        closeResult();
        if (conn == null) {
            return;
        }
        try {
            if (!conn.getAutoCommit()) {
                conn.rollback();
            }
        } catch (SQLException e) {
            e.printStackTrace();
        }
        try {
            conn.close();
        } catch (SQLException e) {
            e.printStackTrace();
        }
    }

    /** 요청·응답에 필요한 만큼의 JSON(객체, 배열, 글자, 수, 참·거짓, null). */
    static final class Json {
        private final String text;
        private int at;

        private Json(String text) {
            this.text = text;
        }

        @SuppressWarnings("unchecked")
        static Map<String, Object> object(String line) {
            Json json = new Json(line);
            Object value = json.value();
            json.space();
            if (!(value instanceof Map) || json.at != line.length()) {
                throw new IllegalArgumentException("JSON 객체가 아닙니다");
            }
            return (Map<String, Object>) value;
        }

        private void space() {
            while (at < text.length() && Character.isWhitespace(text.charAt(at))) {
                at++;
            }
        }

        private char next() {
            space();
            if (at >= text.length()) {
                throw new IllegalArgumentException("JSON이 끝났습니다");
            }
            return text.charAt(at);
        }

        private void expect(char c) {
            if (next() != c) {
                throw new IllegalArgumentException("JSON " + at + ": '" + c + "'가 와야 합니다");
            }
            at++;
        }

        private Object value() {
            char c = next();
            if (c == '{') {
                Map<String, Object> map = new LinkedHashMap<String, Object>();
                at++;
                if (next() == '}') {
                    at++;
                    return map;
                }
                while (true) {
                    String key = string();
                    expect(':');
                    map.put(key, value());
                    if (next() == ',') {
                        at++;
                        continue;
                    }
                    expect('}');
                    return map;
                }
            }
            if (c == '[') {
                List<Object> list = new ArrayList<Object>();
                at++;
                if (next() == ']') {
                    at++;
                    return list;
                }
                while (true) {
                    list.add(value());
                    if (next() == ',') {
                        at++;
                        continue;
                    }
                    expect(']');
                    return list;
                }
            }
            if (c == '"') {
                return string();
            }
            if (text.startsWith("true", at)) {
                at += 4;
                return Boolean.TRUE;
            }
            if (text.startsWith("false", at)) {
                at += 5;
                return Boolean.FALSE;
            }
            if (text.startsWith("null", at)) {
                at += 4;
                return null;
            }
            int start = at;
            while (at < text.length() && "+-0123456789.eE".indexOf(text.charAt(at)) >= 0) {
                at++;
            }
            if (start == at) {
                throw new IllegalArgumentException("JSON " + at + ": 값이 와야 합니다");
            }
            String number = text.substring(start, at);
            return number.matches("-?\\d+") ? (Object) Long.valueOf(number) : (Object) Double.valueOf(number);
        }

        private String string() {
            expect('"');
            StringBuilder out = new StringBuilder();
            while (true) {
                if (at >= text.length()) {
                    throw new IllegalArgumentException("JSON 글자가 끝나지 않았습니다");
                }
                char c = text.charAt(at++);
                if (c == '"') {
                    return out.toString();
                }
                if (c != '\\') {
                    out.append(c);
                    continue;
                }
                char e = text.charAt(at++);
                switch (e) {
                    case 'n': out.append('\n'); break;
                    case 'r': out.append('\r'); break;
                    case 't': out.append('\t'); break;
                    case 'b': out.append('\b'); break;
                    case 'f': out.append('\f'); break;
                    case 'u':
                        out.append((char) Integer.parseInt(text.substring(at, at + 4), 16));
                        at += 4;
                        break;
                    default: out.append(e);
                }
            }
        }

        static String write(Object value) {
            StringBuilder out = new StringBuilder();
            write(out, value);
            return out.toString();
        }

        private static void write(StringBuilder out, Object value) {
            if (value == null) {
                out.append("null");
            } else if (value instanceof String) {
                quote(out, (String) value);
            } else if (value instanceof Boolean || value instanceof Number) {
                out.append(value);
            } else if (value instanceof Map) {
                out.append('{');
                boolean first = true;
                for (Map.Entry<?, ?> entry : ((Map<?, ?>) value).entrySet()) {
                    if (!first) {
                        out.append(',');
                    }
                    first = false;
                    quote(out, entry.getKey().toString());
                    out.append(':');
                    write(out, entry.getValue());
                }
                out.append('}');
            } else if (value instanceof List) {
                out.append('[');
                boolean first = true;
                for (Object item : (List<?>) value) {
                    if (!first) {
                        out.append(',');
                    }
                    first = false;
                    write(out, item);
                }
                out.append(']');
            } else {
                quote(out, value.toString());
            }
        }

        private static void quote(StringBuilder out, String text) {
            out.append('"');
            for (int i = 0; i < text.length(); i++) {
                char c = text.charAt(i);
                switch (c) {
                    case '"': out.append("\\\""); break;
                    case '\\': out.append("\\\\"); break;
                    case '\n': out.append("\\n"); break;
                    case '\r': out.append("\\r"); break;
                    case '\t': out.append("\\t"); break;
                    default:
                        if (c < 0x20) {
                            out.append(String.format("\\u%04x", (int) c));
                        } else {
                            out.append(c);
                        }
                }
            }
            out.append('"');
        }
    }
}
