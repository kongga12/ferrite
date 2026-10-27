"""Ferrite 노트북 커널 사이드카.

jupyter_client로 Jupyter 커널을 띄우고 Ferrite와는 표준 입출력의 한 줄 JSON으로 주고받는다.
Python 3.8 문법만 쓴다(사용자의 Python에서 돈다).

요청: {"method": "start", "params": {"kernel", "cwd"}}, {"method": "execute", "params": {"id", "code"}},
{"method": "data", "params": {"id", "kind"(frames·page), "name", "start", "stop", "sort", "ascending"}},
{"method": "interrupt"}. 입력이 닫히면 커널을 끄고 끝난다.
알림: started {name, display_name, language, missing}, output {id, output(nbformat 출력)}, clear {id},
done {id, execution_count, status(ok·error·aborted·dead)}, value {id, kind, value 또는 error},
error {kind, message}(보낸 뒤 끝난다).
"""

import ast
import json
import os
import queue
import sys
import threading
from collections import deque

# JSON은 원래 표준 출력의 복제본에만 쓴다(복제본은 자식에게 물려주지 않는다). print·로그와 커널
# 프로세스의 출력은 표준 오류로 보낸다.
_out = os.fdopen(os.dup(1), "w", encoding="ascii", newline="\n")
os.dup2(2, 1)
sys.stdout = sys.stderr

commands = queue.Queue()

# 커널 안에서 도는 DataFrame 도우미. 조용한 실행(번호·출력·기록 없음)으로 정의하고 user_expressions로
# 부른다. 사용자 이름공간에는 `_ferrite` 하나만 둔다. pandas를 가져오지 않은 커널에서는 가져오지 않는다.
KERNEL_HELPER = """
class _ferrite:
    @staticmethod
    def frames():
        import json, sys
        pd = sys.modules.get("pandas")
        if pd is None:
            return "[]"
        return json.dumps([
            {"name": k, "rows": int(v.shape[0]), "columns": int(v.shape[1])}
            for k, v in list(get_ipython().user_ns.items())
            if not k.startswith("_") and isinstance(v, pd.DataFrame)
        ])

    @staticmethod
    def page(name, start, stop, sort, ascending):
        import json
        frame = get_ipython().user_ns[name]
        if sort is not None:
            # 열 이름·색인이 겹쳐도 되게 자리로 정렬한다. 섞인 형식이면 글자로 비교한다.
            # ponytail: 페이지마다 다시 정렬한다. 수백만 행에서 느리면 정렬 순서를 캐시한다.
            key = frame.iloc[:, sort].reset_index(drop=True)
            try:
                order = key.sort_values(ascending=ascending, kind="stable").index
            except TypeError:
                order = key.astype(str).sort_values(ascending=ascending, kind="stable").index
            frame = frame.iloc[order]
        part = frame.iloc[start:stop]
        data = part.to_json(orient="values", date_format="iso", double_precision=15, default_handler=str)
        return json.dumps({
            "rows": int(frame.shape[0]),
            "columns": [str(c) for c in frame.columns],
            "dtypes": [str(t) for t in frame.dtypes],
            "index": [str(i) for i in part.index],
            "data": json.loads(data),
        })
"""


def send(event, **fields):
    fields["event"] = event
    _out.write(json.dumps(fields) + "\n")
    _out.flush()


def read_commands():
    # 표준 입력은 UTF-8이다(Windows에서 sys.stdin은 지역 코드 페이지로 읽는다).
    for line in sys.stdin.buffer:
        try:
            commands.put(json.loads(line.decode("utf-8")))
        except ValueError:
            pass
    commands.put(None)


def nbformat_output(msg_type, content):
    """iopub 메시지를 nbformat 출력으로. 셀 출력이 아니면 None."""
    if msg_type == "stream":
        return {"output_type": "stream", "name": content["name"], "text": content["text"]}
    if msg_type in ("display_data", "execute_result"):
        output = {
            "output_type": msg_type,
            "data": content.get("data", {}),
            "metadata": content.get("metadata", {}),
        }
        if msg_type == "execute_result":
            output["execution_count"] = content.get("execution_count")
        return output
    if msg_type == "error":
        return {
            "output_type": "error",
            "ename": content["ename"],
            "evalue": content["evalue"],
            "traceback": content["traceback"],
        }
    return None


class Session:
    def __init__(self):
        self.km = None
        self.kc = None
        # 실행 중에 온 요청(실행이 끝난 뒤 차례로 처리한다).
        self.pending = deque()

    def next_command(self):
        return self.pending.popleft() if self.pending else commands.get()

    def start(self, params):
        from jupyter_client import KernelManager
        from jupyter_client.kernelspec import KernelSpecManager

        specs = KernelSpecManager().get_all_specs()
        name = params.get("kernel") or "python3"
        missing = None
        if name not in specs:
            # 노트북의 커널이 이 컴퓨터에 없으면 이 Python의 커널로 띄운다.
            missing, name = name, "python3"
            if name not in specs:
                raise LookupError("no kernel: %s" % missing)
        self.km = KernelManager(kernel_name=name)
        self.km.start_kernel(cwd=params.get("cwd") or None, stdout=sys.stderr, stderr=sys.stderr)
        self.kc = self.km.client()
        self.kc.start_channels()
        self.kc.wait_for_ready(timeout=120)
        spec = self.km.kernel_spec
        send(
            "started",
            name=name,
            display_name=spec.display_name,
            language=spec.language,
            missing=missing,
        )

    def take_commands(self):
        """실행 중에 온 요청: 멈춤은 곧바로 하고 나머지는 미룬다. 입력이 닫혔으면 False."""
        while True:
            try:
                command = commands.get_nowait()
            except queue.Empty:
                return True
            if command is None:
                self.pending.appendleft(None)
                return False
            if command.get("method") == "interrupt":
                self.km.interrupt_kernel()
            else:
                self.pending.append(command)

    def execute(self, params):
        cell = params["id"]
        if self.kc is None:
            send("done", id=cell, execution_count=None, status="aborted")
            return
        msg_id = self.kc.execute(params["code"], allow_stdin=False)
        # clear_output(wait=True)는 다음 출력이 올 때 지운다.
        clear_wait = False
        while True:
            if not self.take_commands():
                return
            try:
                msg = self.kc.get_iopub_msg(timeout=0.1)
            except queue.Empty:
                if not self.km.is_alive():
                    send("done", id=cell, execution_count=None, status="dead")
                    return
                continue
            if msg["parent_header"].get("msg_id") != msg_id:
                continue
            msg_type, content = msg["msg_type"], msg["content"]
            if msg_type == "status" and content["execution_state"] == "idle":
                break
            if msg_type == "clear_output":
                if content.get("wait"):
                    clear_wait = True
                else:
                    send("clear", id=cell)
                continue
            output = nbformat_output(msg_type, content)
            if output is not None:
                if clear_wait:
                    send("clear", id=cell)
                    clear_wait = False
                send("output", id=cell, output=output)
        reply = self.reply(msg_id)
        status = reply.get("status", "error")
        send("done", id=cell, execution_count=reply.get("execution_count"), status=status)
        if status != "ok":
            # Jupyter처럼 오류가 나면 줄 서 있던 실행은 하지 않는다.
            for command in list(self.pending):
                if command is not None and command.get("method") == "execute":
                    self.pending.remove(command)
                    send("done", id=command["params"]["id"], execution_count=None, status="aborted")

    def data(self, params):
        """DataFrame 목록이나 한 페이지를 답한다. 실패해도 사이드카는 끝나지 않는다."""
        request, kind = params["id"], params.get("kind")
        try:
            if self.kc is None:
                raise RuntimeError("kernel is not running")
            if kind == "page":
                args = (
                    str(params["name"]),
                    int(params["start"]),
                    int(params["stop"]),
                    None if params.get("sort") is None else int(params["sort"]),
                    bool(params.get("ascending", True)),
                )
                expression = "_ferrite.page(%s)" % ", ".join(repr(a) for a in args)
            else:
                expression = "_ferrite.frames()"
            msg_id = self.kc.execute(
                KERNEL_HELPER,
                silent=True,
                store_history=False,
                user_expressions={"value": expression},
                allow_stdin=False,
            )
            result = self.reply(msg_id, timeout=60).get("user_expressions", {}).get("value", {})
            if result.get("status") != "ok":
                raise RuntimeError("%s: %s" % (result.get("ename"), result.get("evalue")))
            # 값은 파이썬 글자의 repr로 온다.
            value = json.loads(ast.literal_eval(result["data"]["text/plain"]))
        except Exception as e:
            send("value", id=request, kind=kind, error=str(e))
            return
        send("value", id=request, kind=kind, value=value)

    def reply(self, msg_id, timeout=10):
        while True:
            try:
                msg = self.kc.get_shell_msg(timeout=timeout)
            except queue.Empty:
                return {}
            if msg["parent_header"].get("msg_id") == msg_id:
                return msg["content"]

    def shutdown(self):
        if self.kc is not None:
            self.kc.stop_channels()
        if self.km is not None and self.km.has_kernel:
            self.km.shutdown_kernel()


def main():
    threading.Thread(target=read_commands, daemon=True).start()
    try:
        import jupyter_client  # noqa: F401
    except ImportError as e:
        send("error", kind="missing", message=str(e))
        return
    session = Session()
    try:
        while True:
            command = session.next_command()
            if command is None:
                break
            method, params = command.get("method"), command.get("params") or {}
            if method == "start":
                session.start(params)
            elif method == "execute":
                session.execute(params)
            elif method == "data":
                session.data(params)
            elif method == "interrupt" and session.km is not None:
                session.km.interrupt_kernel()
    except Exception as e:
        send("error", kind=type(e).__name__, message=str(e))
    finally:
        session.shutdown()


main()
