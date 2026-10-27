"""Ferrite 노트북 커널 사이드카.

jupyter_client로 Jupyter 커널을 띄우고 Ferrite와는 표준 입출력의 한 줄 JSON으로 주고받는다.
Python 3.8 문법만 쓴다(사용자의 Python에서 돈다).

요청: {"method": "start", "params": {"kernel", "cwd"}}, {"method": "execute", "params": {"id", "code"}},
{"method": "interrupt"}. 입력이 닫히면 커널을 끄고 끝난다.
알림: started {name, display_name, language, missing}, output {id, output(nbformat 출력)}, clear {id},
done {id, execution_count, status(ok·error·aborted·dead)}, error {kind, message}(보낸 뒤 끝난다).
"""

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

    def reply(self, msg_id):
        while True:
            try:
                msg = self.kc.get_shell_msg(timeout=10)
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
            elif method == "interrupt" and session.km is not None:
                session.km.interrupt_kernel()
    except Exception as e:
        send("error", kind=type(e).__name__, message=str(e))
    finally:
        session.shutdown()


main()
