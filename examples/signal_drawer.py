import pandas as pd
import sys
import matplotlib.pyplot as plt

fname = sys.argv[1]
df = pd.read_csv(fname)


fig = plt.gcf()

VALUED16_DATA = ["CPU ADDR", "SCREEN"]
VALUED8_DATA = [ "CPU DATA"]

nb_sig = len(df.columns)
for i, col in enumerate(df):
	values = df[col].values
	nb_events = len(values)
	plt.subplot(nb_sig,1,i+1)

	if col not in VALUED8_DATA and col not in VALUED16_DATA:
		plt.plot(values, 
				drawstyle="steps", 
				linewidth=2,
				c="black")
	else:
		for j, v in enumerate(values):
			if col in VALUED8_DATA:
				t = "0x%02x" % v
			else:
				t = "0x%04x" % v
			if j % 2:
				t = ""
			plt.text(j, 0.5, t, rotation="vertical")
	plt.xlim(-1, len(values))

	plt.ylabel(col, rotation=0, fontweight="bold")


	for j in range(len(values))[::2]:
		plt.vlines(
			j,
			-0.25,
			1.25,
			"gray",
			":"
		)

	ax = plt.gca()
	ax.spines['top'].set_visible(False)
	ax.spines['right'].set_visible(False)
	ax.spines['bottom'].set_visible(False)
	ax.spines['left'].set_visible(False)
	ax.get_xaxis().set_ticks([])
	ax.get_yaxis().set_ticks([])



fig.subplots_adjust(wspace=0, hspace=0)

plt.show()