//Plugin: BestAzon
//Compatible with Version: 2.0.0
//DO NOT CHANGE ANYTHING IN THIS FILE OR THE PLUGIN MAY BREAK OR BEHAVE UNPREDICTABLY
var Redirect_Service_Base_URL = "http://lnks.io/r.php"; //Set redirect service URL
var BestAzon_Version = "03102017";
var visitor_country; //Set variable to store country code
//list of domains for which the redirect should work
var Valid_Domains = [
    "amazon",
    "amzn"
];
var keyword_array = {};
var subDomain = "";
var isHTTP = 0;
var BestAzon_Debug = 0;
var optionsParameter = ""; // Array to store all the options for the plugin
var TrackingId;
var clientId;

if (window.location.href.indexOf("BestAzon_Debug=1") > -1)
{
    BestAzon_Debug = 1;
    Write_To_Console("BestAzon Version is " + BestAzon_Version);
    Write_To_Console(BestAzon_Configuration);
}
checkjQuery();


//Check if its a HTTPs site
if (location.protocol === 'http:')
{
    Write_To_Console("Location Protocol is " + location.protocol);
    isHTTP = 1;
}
//load all plugin options in URL parameter format
for (var option_key in BestAzon_Configuration)
{
    if (BestAzon_Configuration[option_key] != "" && typeof BestAzon_Configuration[option_key] != "undefined")
    {
        optionsParameter += "&" + option_key + "=" + encodeURIComponent(BestAzon_Configuration[option_key]);
    }
    else
    {}
}
if (typeof BestAzon_Configuration['Conf_Link_Keywords'] == "undefined") //checking if Link Keywords are defined
{
    Write_To_Console("Link keywords not defined. Setting to blank");
    BestAzon_Configuration['Conf_Link_Keywords'] = "";
}
if (BestAzon_Configuration['Conf_Link_Keywords'].length > 0)
{
    keyword_array = BestAzon_Configuration['Conf_Link_Keywords'].split(' ');
}

//Get GA account and the client ID
if (BestAzon_Configuration['Conf_GA_Tracking'] != 2)
	{
	var gaObj = window['GoogleAnalyticsObject'];
	Write_To_Console(gaObj);
		if (typeof gaObj == "undefined") //checking if gaObj was found
			{
				Write_To_Console("Ga Object Not Found");
				clientId = "";
				TrackingId = "";
			}
		else 
			{
			window[gaObj](function(tracker) {
				clientId = tracker.get('clientId');
				Write_To_Console(clientId);
				TrackingId = tracker.get('trackingId');
				Write_To_Console(TrackingId);
			});
			}
	}
else
{
	Write_To_Console("USer has chosen not to use Google Analytics tracking");
	clientId = "";
	TrackingId = "";
}


//Get Country Code and if successful prepare the URL
if (isHTTP != 1)
{
    // page is not HTTP, cannot get the location without paying for SSL services! :-)
    Write_To_Console("This is SSL page. The location will be determined by lnks.io");
    visitor_country = '';
    prepareURL(); //prepare the URL
}
else
{
    jQuery.ajax(
        {
            url: "http://ipinfo.io",
            dataType: "json"
        }, "jsonp")
        .always(function(response) {})
        .done(function(response)
        {
            visitor_country = response.country;
            if (typeof visitor_country == 'undefined')
            {
                visitor_country = '';
            }
            prepareURL(); //prepare the URL
        })
        .fail(function(response)
        {
            Write_To_Console("Primary Geolocation failed, trying second service");
            fallBackGeoLocation(); //If unsuccessful, try second service
        });
}
//Function to find the base domain from the URL
function extractDomain(url)
{
    var domain;
    //find & remove protocol (http, ftp, etc.) and get domain
    if (url.indexOf("://") > -1)
    {
        url = url.split('/')[2];
    }
    else
    {
        url = url.split('/')[0];
    }
    //find & remove port number
    url = url.split(':')[0];
    //find & remove protocol www
    if (url.indexOf("www.") > -1)
    {
        url = url.split('www.')[1];
    }
    var re = new RegExp('.*(co.uk|co.jp|com|ca|de|fr|co.au|in|cn|it|mx|es|com.br|to)', 'i');
    var domainmatch = url.match(re);
    if (domainmatch !== null)
    {
        url = url.replace("." + domainmatch[1], '');
        domain = url.split('.')
            .pop()
            .toLowerCase();
        if (url.split('.')
            .length > 1)
        {
            subDomain = url.replace("." + domain, '')
                .toLowerCase();
        }
        else
        {
            subDomain = "";
        }
        return domain;
    }
    else
    {
        return "DomainNotFound";
    }
}
//Check if domain name matches the redirect domains.
function Valid_URL(url)
{
    URL_Domain = extractDomain(url);
    //if the domain is a listed domain, return true else false
    if (jQuery.inArray(URL_Domain, Valid_Domains) != -1 && subDomain == "")
    {
        return 1;
    }
    else
    {
        var arrayLength = keyword_array.length;
		for (var key = 0; key < arrayLength; key++) 
        {
            if (url.toLowerCase()
                .indexOf(keyword_array[key].toLowerCase()) > -1)
            {
                return 1;
            }
        }
        return 0;
    }
}

function prepareURL()
{
    //Once the document loads, find all links and replace the URL if its a listed domain
    jQuery(document)
        .ready(function()
        {
            var x = document.getElementsByTagName('a');
            var i;
            for (i = 0; i < x.length; i++)
            {
                // find the domain of the URL
                if (Valid_URL(x[i].href))
                {
                    var url_to_redirect = Redirect_Service_Base_URL + "?refURL=" + encodeURIComponent(window.location.href) + "&destURL=" + encodeURIComponent(x[i].href) + optionsParameter + "&Country=" + visitor_country+ "&TrackID=" + TrackingId+ "&clickID=" + clientId;
                    if (BestAzon_Configuration['Conf_Hide_Redirect_Link'] == 1)
                    {
                        Write_To_Console("Admin has chosen to use JS for redirects");
                        // Attaching the redirect URL on click event
                        x[i].onclick = function()
                        {
                            var url_to_redirect = Redirect_Service_Base_URL + "?refURL=" + encodeURIComponent(window.location.href) + "&destURL=" + encodeURIComponent(this.href) + optionsParameter + "&Country=" + visitor_country+ "&TrackID=" + TrackingId+ "&clickID=" + clientId;
                            if (BestAzon_Configuration['Conf_New_Window'] == 1)
                            {
                                Write_To_Console("Admin has chosen to open in new window");
                                window.open(
                                    url_to_redirect,
                                    '_blank' // <- To open in a new window.
                                );
                            }
                            else
                            {
                                Write_To_Console("Admin has chosen to open in same window");
                                window.location.href = url_to_redirect; // if https, load in the window
                            }
                            return false;
                        };
                    }
                    //If admin has chosen so, replace the links in the URL with the redirect link
                    else
                    {
                        Write_To_Console("Admin has chosen to change the URLs for redirect");
                        x[i].href = url_to_redirect;
                    }
                    //Add the css classes to the link
                    x[i].className += BestAzon_Configuration['' + 'Conf_Custom_Class'];
                    //add the target attribute to the link
                    if (BestAzon_Configuration['Conf_New_Window'] == 1)
                    {
                        x[i].target = "_blank";
                    }
                    else if (BestAzon_Configuration['Conf_New_Window'] == 2)
                    {
                        x[i].target = "_self"; // if https, load in the window
                    }
                    //Add the nofollow attribute, if needed
                    if (BestAzon_Configuration['Conf_Link_Follow'] == 1)
                    {
                        x[i].rel = "nofollow";
                    }
                }
            }
        });
}

function fallBackGeoLocation()
{
    jQuery.ajax(
        {
            url: "http://freegeoip.net/json/",
            dataType: "json"
        }, "jsonp")
        .always(function(response)
        {
            visitor_country = response.country_code;
            if (typeof visitor_country == 'undefined')
            {
                visitor_country = '';
            }
            //Once the document loads, find all links and replace the URL if its a listed domain
            prepareURL();
        })
        .done(function(response) {})
        .fail(function(response)
        {
            Write_To_Console("Secondary Geolocation service also failed... don't worry, we will get it done through BestAzon!");
        });
}

function Write_To_Console(console_message)
{
    if (BestAzon_Debug == 1)
    {
        console.log(console_message);
    }
}
// Checking for jQuery
function checkjQuery()
{
	if (typeof jQuery != "undefined")
    {
        Write_To_Console("jQuery loaded, all set!");
    }
    else
    {
        Write_To_Console("jQuery not loaded, loading now");
		// get some kind of XMLHttpRequest
		var xhrObj = new XMLHttpRequest();
		// open and send a synchronous request
		xhrObj.open('GET', "https://ajax.googleapis.com/ajax/libs/jquery/1.12.4/jquery.min.js", false);
		xhrObj.send('');
		// add the returned content to a newly created script tag
		var jqueryscript = document.createElement('script');
		jqueryscript.type = "text/javascript";
		jqueryscript.text = xhrObj.responseText;
		document.getElementsByTagName('head')[0].appendChild(jqueryscript);
		Write_To_Console("jQuery Loaded!");
    }
}