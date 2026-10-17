"""Fixture-only raw TLS/TCP passthrough for official Runtime native exit tests.

No WebSocket implementation participates: upstream HTTP status and close-frame
bytes pass unchanged, including when a late input/resize write fails. No payload
or Authorization is retained/logged. Each direction ends independently.
"""
import asyncio
from official_runtime import WireFixture

class ClosePreservingRuntimeGateway(WireFixture):
    def __init__(self, root, harness):
        self.harness=harness
        self.connections=set()
        self.close_trace=[]
        super().__init__(root)
        self.credential=harness.credential

    async def _start(self):
        self.server=await asyncio.start_server(self.relay,'127.0.0.1',0,ssl=self.context)
        self.origin='https://127.0.0.1:'+str(self.server.sockets[0].getsockname()[1])

    async def relay(self, downstream, downstream_writer):
        task=asyncio.current_task();self.connections.add(task)
        upstream_writer=None;tasks=[]
        try:
            upstream,upstream_writer=await asyncio.open_connection('127.0.0.1',self.harness.port)
            async def pump(reader,writer,direction):
                count=0
                try:
                    while True:
                        block=await reader.read(65536)
                        if not block:return
                        writer.write(block)
                        await writer.drain();count+=len(block)
                except (ConnectionError,OSError):
                    return
                finally:
                    self.close_trace.append({'direction':direction,'bytes':count})
                    if writer.can_write_eof():
                        try:writer.write_eof()
                        except (ConnectionError,OSError):pass
            tasks=[asyncio.create_task(pump(downstream,upstream_writer,'upstream')),
                   asyncio.create_task(pump(upstream,downstream_writer,'downstream'))]
            await asyncio.wait(tasks,return_when=asyncio.FIRST_COMPLETED)
            # A failed late write must not cancel the other side's final close.
            try:await asyncio.wait_for(asyncio.gather(*tasks),2)
            except asyncio.TimeoutError:pass
        finally:
            for pending in tasks:pending.cancel()
            if tasks:await asyncio.gather(*tasks,return_exceptions=True)
            for writer in (upstream_writer,downstream_writer):
                if writer:
                    writer.close()
                    try:await asyncio.wait_for(writer.wait_closed(),1)
                    except (asyncio.TimeoutError,ConnectionError,OSError):pass
            self.connections.discard(task)

    def close(self):
        async def stop():
            self.server.close();await self.server.wait_closed()
            tasks=list(self.connections)
            for task in tasks:task.cancel()
            if tasks:await asyncio.gather(*tasks,return_exceptions=True)
        try:self.call(stop())
        finally:
            self.loop.call_soon_threadsafe(self.loop.stop);self.thread.join(timeout=5);self.loop.close()
