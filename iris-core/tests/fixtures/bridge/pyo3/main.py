# PyO3 imports — Python side
from mymodule import hello, Config

msg = hello("World")
cfg = Config()
debug = cfg.is_debug()
