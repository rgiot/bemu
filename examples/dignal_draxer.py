import pandas as pd
import sys
import matplotlib.pyplot as plt

fname = sys.argv[1]
df = pd.read_csv(fname)


fig = plt.gcf()

nb_sig = len(df.columns)
for i, col in enumerate(df):
	values = df[col].values
	nb_events = len(values)

	# Handle real data
	plt.subplot(nb_sig,1,i+1)
	plt.plot(values, 
			drawstyle="steps", 
			linewidth=2,
			c="black")
	plt.ylabel(col, rotation=0, fontweight="bold")


	for j in range(len(values)):
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