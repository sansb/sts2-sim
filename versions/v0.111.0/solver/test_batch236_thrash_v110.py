"""#138 Batch 236 / #872: v0.110.1 Thrash live-divergence repair.

Trimmed by #2827 item F: the tests here that exercised the deleted Python
simulator went with it; the ones that remain never reached it.
"""

from pathlib import Path


HERE = Path(__file__).parent
